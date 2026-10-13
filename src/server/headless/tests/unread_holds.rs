//! `agent.mark_unread` against attached clients: when a held completion
//! comes back, and when presenting the pane clears it again.

use super::picks::{client_request, error_code};
use super::*;
use crate::api::schema::{
    AgentStatus, EventData, Method, PaneTarget, Request, ResponseResult, WorkspaceTarget,
};
use crate::detect::AgentState;
use crate::pane::unread_hold::UnreadHold;

struct Fixture {
    server: HeadlessServer,
    hub: api::EventHub,
    pane: crate::layout::PaneId,
    pane_id: String,
}

/// Two spaces. The first hosts an agent whose completion has been seen.
fn fixture() -> Fixture {
    let hub = api::EventHub::default();
    let mut server = test_headless_server_with_event_hub(hub.clone());
    let agents = crate::workspace::Workspace::test_new("agents");
    let pane = agents.tabs[0].root_pane;
    server.app.state.workspaces = vec![agents, crate::workspace::Workspace::test_new("other")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let pane_id = server.app.public_pane_id(0, pane).unwrap();
    let mut fixture = Fixture {
        server,
        hub,
        pane,
        pane_id,
    };
    fixture.agent_state(AgentState::Working);
    fixture.agent_state(AgentState::Idle);
    assert_eq!(fixture.status(), AgentStatus::Idle);
    fixture
}

impl Fixture {
    fn agent_state(&mut self, state: AgentState) {
        self.server
            .app
            .state
            .handle_app_event(AppEvent::StateChanged {
                pane_id: self.pane,
                agent: Some(crate::detect::Agent::Pi),
                state,
                visible_blocker: false,
                visible_working: state == AgentState::Working,
                process_exited: false,
                observed_at: Instant::now(),
            });
    }

    fn agent(&self) -> api::schema::AgentInfo {
        self.server
            .app
            .session_snapshot()
            .agents
            .into_iter()
            .find(|agent| agent.pane_id == self.pane_id)
            .expect("the agent")
    }

    fn status(&self) -> AgentStatus {
        self.agent().agent_status
    }

    fn hold(&self) -> Option<UnreadHold> {
        self.server.app.state.workspaces[0]
            .pane_state(self.pane)
            .unwrap()
            .unread_hold
    }

    fn mark(&mut self, client_id: u64) -> String {
        let response = client_request(
            &mut self.server,
            client_id,
            Method::AgentMarkUnread(PaneTarget {
                pane_id: self.pane_id.clone(),
            }),
        );
        self.server.settle_unread_holds();
        response
    }

    fn api_mark(&mut self, pane_id: &str) -> String {
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        self.server
            .handle_api_request_with_shutdown_check(api::ApiRequestMessage {
                request: Request {
                    id: "api".into(),
                    method: Method::AgentMarkUnread(PaneTarget {
                        pane_id: pane_id.into(),
                    }),
                },
                respond_to,
                response_write_complete: None,
            });
        response_rx.try_recv().expect("mark_unread response")
    }

    /// `pane focus` on the agent's pane from the socket API, as a script
    /// sends it.
    fn api_focus(&mut self) -> String {
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        self.server
            .handle_api_request_with_shutdown_check(api::ApiRequestMessage {
                request: Request {
                    id: "api".into(),
                    method: Method::PaneFocus(PaneTarget {
                        pane_id: self.pane_id.clone(),
                    }),
                },
                respond_to,
                response_write_complete: None,
            });
        response_rx.try_recv().expect("pane.focus response")
    }

    /// Moves one client to a space, as its own navigation does.
    fn visit(&mut self, client_id: u64, workspace_index: usize) {
        let workspace_id = self.server.app.public_workspace_id(workspace_index);
        client_request(
            &mut self.server,
            client_id,
            Method::WorkspaceFocus(WorkspaceTarget { workspace_id }),
        );
        self.server.settle_unread_holds();
    }

    fn focus(&mut self, client_id: u64, focused: bool) {
        self.server
            .handle_server_event(ServerEvent::ClientShellFocus { client_id, focused });
    }
}

#[tokio::test]
async fn a_marked_completion_returns_when_its_client_leaves_and_clears_on_return() {
    let mut fixture = fixture();
    let (_control, _) = connect_matching_test_shell(&mut fixture.server, 61);
    let acknowledged = fixture.agent().state_change_seq;
    let events_before = fixture.hub.current_sequence();

    let response = fixture.mark(61);
    let response: api::schema::SuccessResponse = serde_json::from_str(&response).expect(&response);
    let ResponseResult::AgentInfo { agent } = response.result else {
        panic!("expected the agent: {response:?}");
    };
    assert_eq!(agent.pane_id, fixture.pane_id);
    assert_eq!(agent.agent_status, AgentStatus::Idle);
    // Held, but nothing shows while the client stays on the pane.
    assert_eq!(
        fixture.hold(),
        Some(UnreadHold::Armed {
            client_id: Some(61)
        })
    );
    assert_eq!(fixture.status(), AgentStatus::Idle);
    // Losing window focus is not leaving the pane.
    fixture.focus(61, false);
    fixture.focus(61, true);
    assert_eq!(fixture.status(), AgentStatus::Idle);

    fixture.visit(61, 1);
    let restored = fixture.agent();
    assert_eq!(restored.agent_status, AgentStatus::Done);
    assert!(restored.state_change_seq > acknowledged);
    assert_eq!(restored.completion_seq, Some(restored.state_change_seq));
    assert_eq!(fixture.hold(), Some(UnreadHold::Restored));
    let announced = fixture
        .hub
        .events_after(events_before)
        .into_iter()
        .filter_map(|(_, event)| match event.data {
            EventData::PaneAgentStatusChanged {
                pane_id,
                agent_status,
                ..
            } => Some((pane_id, agent_status)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(announced, [(fixture.pane_id.clone(), AgentStatus::Done)]);

    // Marks that cover the whole presented tab pass over the held pane.
    fixture.server.app.state.active = Some(0);
    fixture.server.sync_foreground_client_state();
    assert!(!fixture.server.settle_unread_holds());
    assert_eq!(fixture.status(), AgentStatus::Done);
    assert_eq!(error_code(&fixture.mark(61)), "agent_not_idle");

    fixture.visit(61, 0);
    assert_eq!(fixture.status(), AgentStatus::Idle);
    assert_eq!(fixture.hold(), None);
    assert!(!fixture.server.app.state.fork.unread_holds_pending);
    // The requeue stays: the sequence is the restored one.
    assert_eq!(fixture.agent().state_change_seq, restored.state_change_seq);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn only_the_marking_client_leaving_restores_and_only_an_arrival_clears() {
    let mut fixture = fixture();
    let (_first, _) = connect_matching_test_shell(&mut fixture.server, 61);
    let (_second, _) = connect_matching_test_shell(&mut fixture.server, 62);

    fixture.mark(61);
    // Another client moving about does not release the first client's hold.
    fixture.visit(62, 1);
    fixture.visit(62, 0);
    assert_eq!(fixture.status(), AgentStatus::Idle);

    // The marker leaves while the other client still sits on the pane.
    fixture.visit(61, 1);
    assert_eq!(fixture.status(), AgentStatus::Done);
    // Sitting there is not arriving: the badge stays for both.
    assert!(!fixture.server.settle_unread_holds());
    assert_eq!(fixture.status(), AgentStatus::Done);

    // Its window coming back to the front is an arrival.
    fixture.focus(62, false);
    assert_eq!(fixture.status(), AgentStatus::Done);
    fixture.focus(62, true);
    assert_eq!(fixture.status(), AgentStatus::Idle);
    assert_eq!(fixture.hold(), None);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn an_unfocused_window_does_not_clear_a_restored_completion() {
    let mut fixture = fixture();
    let (_first, _) = connect_matching_test_shell(&mut fixture.server, 61);
    let (_second, _) = connect_matching_test_shell(&mut fixture.server, 62);
    fixture.visit(62, 1);
    fixture.focus(62, false);

    fixture.mark(61);
    fixture.visit(61, 1);
    assert_eq!(fixture.status(), AgentStatus::Done);

    // The background window navigates to the pane, unseen by anyone.
    fixture.visit(62, 0);
    assert_eq!(fixture.status(), AgentStatus::Done);
    fixture.focus(62, true);
    assert_eq!(fixture.status(), AgentStatus::Idle);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn a_marker_that_detaches_or_switches_endpoint_has_left() {
    let mut detached = fixture();
    let (_control, _) = connect_matching_test_shell(&mut detached.server, 61);
    detached.mark(61);
    detached
        .server
        .handle_server_event(ServerEvent::ClientDisconnected { client_id: 61 });
    assert_eq!(detached.status(), AgentStatus::Done);
    shutdown_test_runtimes(&mut detached.server);

    let mut switched = fixture();
    let (_control, _) = connect_matching_test_shell(&mut switched.server, 61);
    switched.mark(61);
    // Presenting another endpoint deactivates this one's surface.
    switched
        .server
        .set_client_shell_surface_active(61, false)
        .unwrap();
    assert!(switched.server.settle_unread_holds());
    assert_eq!(switched.status(), AgentStatus::Done);
    // Coming back to this endpoint presents the pane again.
    switched
        .server
        .set_client_shell_surface_active(61, true)
        .unwrap();
    assert!(switched.server.settle_unread_holds());
    assert_eq!(switched.status(), AgentStatus::Idle);
    shutdown_test_runtimes(&mut switched.server);
}

#[tokio::test]
async fn a_mark_from_the_api_is_held_by_whoever_presents_the_pane() {
    // Nobody presents the pane, so the completion is back at once.
    let mut unattended = fixture();
    let pane_id = unattended.pane_id.clone();
    let response = unattended.api_mark(&pane_id);
    assert!(response.contains(r#""type":"agent_info""#), "{response}");
    assert_eq!(unattended.status(), AgentStatus::Done);
    shutdown_test_runtimes(&mut unattended.server);

    let mut presented = fixture();
    let (_first, _) = connect_matching_test_shell(&mut presented.server, 61);
    let (_second, _) = connect_matching_test_shell(&mut presented.server, 62);
    let pane_id = presented.pane_id.clone();
    presented.api_mark(&pane_id);
    assert_eq!(
        presented.hold(),
        Some(UnreadHold::Armed { client_id: None })
    );
    presented.visit(61, 1);
    assert_eq!(presented.status(), AgentStatus::Idle);
    presented.visit(62, 1);
    assert_eq!(presented.status(), AgentStatus::Done);
    shutdown_test_runtimes(&mut presented.server);
}

/// A focus command from the socket API marks its target seen, but passes
/// over a completion `agent.mark_unread` brought back. What clears that one
/// is the client the command brings to the pane.
#[tokio::test]
async fn a_public_focus_clears_a_restored_completion_only_by_bringing_a_client_to_it() {
    // Nobody is attached, so nobody arrives.
    let mut unattended = fixture();
    let pane_id = unattended.pane_id.clone();
    unattended.api_mark(&pane_id);
    assert_eq!(unattended.status(), AgentStatus::Done);
    let response = unattended.api_focus();
    assert!(response.contains(r#""type":"pane_info""#), "{response}");
    assert_eq!(unattended.status(), AgentStatus::Done);
    assert_eq!(unattended.hold(), Some(UnreadHold::Restored));
    shutdown_test_runtimes(&mut unattended.server);

    // One client already sits on the pane, and the other's window is known
    // to be in the background: the command moves that one onto the pane,
    // but neither arrives.
    let mut background = fixture();
    let (_first, _) = connect_matching_test_shell(&mut background.server, 61);
    let (_second, _) = connect_matching_test_shell(&mut background.server, 62);
    background.mark(61);
    background.visit(61, 1);
    background.focus(61, false);
    assert_eq!(background.status(), AgentStatus::Done);
    background.api_focus();
    assert_eq!(background.status(), AgentStatus::Done);
    assert_eq!(background.hold(), Some(UnreadHold::Restored));
    // The moved client presents the pane now: its window coming back to
    // the front is the arrival.
    background.focus(61, true);
    assert_eq!(background.status(), AgentStatus::Idle);
    shutdown_test_runtimes(&mut background.server);

    // A client on another space whose window is not in the background is
    // moved onto the pane, which is an arrival.
    let mut attended = fixture();
    let (_control, _) = connect_matching_test_shell(&mut attended.server, 61);
    attended.mark(61);
    attended.visit(61, 1);
    assert_eq!(attended.status(), AgentStatus::Done);
    attended.api_focus();
    assert_eq!(attended.status(), AgentStatus::Idle);
    assert_eq!(attended.hold(), None);
    shutdown_test_runtimes(&mut attended.server);
}

#[tokio::test]
async fn a_real_lifecycle_change_cancels_the_hold() {
    let mut fixture = fixture();
    let (_control, _) = connect_matching_test_shell(&mut fixture.server, 61);
    fixture.mark(61);
    fixture.agent_state(AgentState::Working);
    assert_eq!(fixture.hold(), None);

    fixture.visit(61, 1);
    assert_eq!(fixture.status(), AgentStatus::Working);
    assert!(!fixture.server.app.state.fork.unread_holds_pending);

    // The same holds once the completion is back.
    fixture.visit(61, 0);
    fixture.agent_state(AgentState::Idle);
    fixture.mark(61);
    fixture.visit(61, 1);
    assert_eq!(fixture.status(), AgentStatus::Done);
    fixture.agent_state(AgentState::Working);
    assert_eq!(fixture.hold(), None);
    assert_eq!(fixture.status(), AgentStatus::Working);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn mark_unread_rejects_what_has_no_acknowledged_completion() {
    let mut fixture = fixture();
    let (_control, _) = connect_matching_test_shell(&mut fixture.server, 61);

    assert_eq!(error_code(&fixture.api_mark("w9:p9")), "pane_not_found");
    let shell_pane = fixture.server.app.state.workspaces[1].tabs[0].root_pane;
    let shell_pane_id = fixture.server.app.public_pane_id(1, shell_pane).unwrap();
    assert_eq!(
        error_code(&fixture.api_mark(&shell_pane_id)),
        "agent_not_found"
    );

    for busy in [AgentState::Working, AgentState::Blocked] {
        fixture.agent_state(busy);
        assert_eq!(error_code(&fixture.mark(61)), "agent_not_idle");
    }
    assert_eq!(fixture.hold(), None);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn a_restored_completion_reaches_clients_as_done_under_a_new_sequence() {
    let mut fixture = fixture();
    let (control, render) = connect_matching_test_shell(&mut fixture.server, 61);
    let seeded = client_shell_snapshot(&control);
    let before = seeded.agents[0].clone();
    assert_eq!(before.agent_status, AgentStatus::Idle);
    assert!(before.priority_key.starts_with("1.3."), "{before:?}");

    fixture.mark(61);
    fixture.visit(61, 1);
    for _ in render.try_iter() {}
    fixture.server.render_and_stream();
    let snapshot = client_shell_snapshot(&control);
    let after = &snapshot.agents[0];

    // Clients show the snapshot status, so every one of them has the badge.
    assert_eq!(after.agent_status, AgentStatus::Done);
    assert_eq!(snapshot.workspaces[0].agent_status, AgentStatus::Done);
    // The restored completion is a new ordering event.
    assert!(after.state_change_seq > before.state_change_seq);
    // It queues as the youngest done agent.
    assert!(after.priority_key.starts_with("1.1."), "{after:?}");
    shutdown_test_runtimes(&mut fixture.server);
}

/// Clients keep no record of what they have shown, so an ordinary completion
/// badge appears and clears by the daemon's seen state alone. A client drives
/// that state with the requests and focus reports it already sends.
#[tokio::test]
async fn a_completion_badge_follows_the_daemons_seen_state_as_a_client_moves() {
    let mut fixture = fixture();
    let (_control, _) = connect_matching_test_shell(&mut fixture.server, 61);
    let finish = |fixture: &mut Fixture| {
        fixture.agent_state(AgentState::Working);
        fixture.agent_state(AgentState::Idle);
    };

    // The client presents the agent's tab: the completion is seen as it lands.
    fixture.focus(61, true);
    finish(&mut fixture);
    assert_eq!(fixture.status(), AgentStatus::Idle);

    // Behind the client's back it is a badge, which going there clears.
    fixture.visit(61, 1);
    finish(&mut fixture);
    assert_eq!(fixture.status(), AgentStatus::Done);
    fixture.visit(61, 0);
    assert_eq!(fixture.status(), AgentStatus::Idle);

    // With the window in the background it is a badge too, until the window
    // comes back to the front.
    fixture.focus(61, false);
    finish(&mut fixture);
    assert_eq!(fixture.status(), AgentStatus::Done);
    fixture.focus(61, true);
    assert_eq!(fixture.status(), AgentStatus::Idle);
    shutdown_test_runtimes(&mut fixture.server);
}

#[tokio::test]
async fn a_drained_pane_death_that_moves_a_client_onto_the_pane_clears_the_hold() {
    let mut fixture = fixture();
    let (_control, _) = connect_matching_test_shell(&mut fixture.server, 61);
    fixture.mark(61);
    fixture.visit(61, 1);
    assert_eq!(fixture.status(), AgentStatus::Done);

    // The space the client went to closes with its only pane, which moves
    // the client back onto the held pane. The loop's batch drain takes the
    // event, and no client event or request follows it.
    let other = fixture.server.app.state.workspaces[1].tabs[0].root_pane;
    fixture
        .server
        .app
        .event_tx
        .try_send(AppEvent::PaneDied {
            pane_id: other,
            exit_reason: crate::platform::ChildExitReason::Exited,
        })
        .unwrap();
    assert!(fixture.server.drain_internal_events_with_forwarding());
    assert_eq!(fixture.status(), AgentStatus::Idle);
    assert_eq!(fixture.hold(), None);
    shutdown_test_runtimes(&mut fixture.server);
}
