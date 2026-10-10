//! The client loop's side of the contract: what `ClientContract::sync` writes
//! to which connection, turn by turn. Transports record instead of sending.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::api::schema::{AgentStatus, ClientJumpOutcome};
use crate::client::endpoint::{EndpointNegotiation, EndpointTransport};
use crate::client::shell::tests::fleet::{agent, fleet, machine, retained};
use crate::client::shell::{ControlOrigin, NAVIGATION_SETTLE_DEADLINE};
use crate::protocol::endpoint::EndpointClientJump;

type Sent = Arc<Mutex<Vec<ClientMessage>>>;

struct Recorder(Sent);

impl EndpointTransport for Recorder {
    fn send(&mut self, message: &ClientMessage) -> std::io::Result<()> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}

/// What a daemon of this build negotiates.
fn negotiation() -> EndpointNegotiation {
    EndpointNegotiation::new(
        crate::server::client_commands::supported_client_shell_method_names()
            .iter()
            .map(|method| (*method).to_owned())
            .collect(),
        Vec::new(),
    )
}

/// The lane requests a connection carried, as `(boot id, request)`.
fn requests(sent: &Sent) -> Vec<(String, Request)> {
    sent.lock()
        .unwrap()
        .iter()
        .map(|message| match message {
            ClientMessage::ClientShellEndpointRequest { boot_id, request } => (
                boot_id.clone(),
                serde_json::from_str(request).expect("lane request"),
            ),
            other => panic!("unexpected message: {other:?}"),
        })
        .collect()
}

fn reports(sent: &Sent) -> Vec<ClientReport> {
    requests(sent)
        .into_iter()
        .map(|(_, request)| match request.method {
            Method::ClientReport(report) => {
                assert!(ClientContract::owns_response(&request.id), "{}", request.id);
                report
            }
            method => panic!("expected a report: {method:?}"),
        })
        .collect()
}

fn panes(report: &ClientReport) -> Vec<(&str, &str)> {
    report
        .agents
        .iter()
        .map(|row| (row.endpoint_id.as_str(), row.agent.pane_id.as_str()))
        .collect()
}

/// A client attached to Local and Build, both connected, presenting Local.
struct Client {
    shell: ClientShellState,
    endpoints: EndpointRegistry,
    contract: ClientContract,
    remote: ClientEndpointId,
    local_sent: Sent,
    remote_sent: Sent,
    now: Instant,
}

impl Client {
    fn new() -> Self {
        let (mut shell, remotes) = fleet(&["Build"]);
        shell.set_snapshot(Box::new(machine(
            "boot-1",
            vec![
                agent("pane_1", AgentStatus::Idle, 1),
                agent("pane_2", AgentStatus::Done, 2),
            ],
        )));
        shell.set_endpoint_snapshot(
            &remotes[0],
            Box::new(machine(
                "build-boot",
                vec![agent("pane_7", AgentStatus::Blocked, 3)],
            )),
        );
        let local_sent = Sent::default();
        let remote_sent = Sent::default();
        let mut endpoints = EndpointRegistry::new(Recorder(local_sent.clone()), 1, negotiation());
        endpoints.insert(
            remotes[0].clone(),
            Recorder(remote_sent.clone()),
            2,
            negotiation(),
            false,
        );
        Self {
            shell,
            endpoints,
            contract: ClientContract::default(),
            remote: remotes[0].clone(),
            local_sent,
            remote_sent,
            now: Instant::now(),
        }
    }

    /// One client-loop turn.
    fn turn(&mut self) {
        self.contract
            .sync(&mut self.shell, &mut self.endpoints, self.now);
    }

    /// The client loop completes the handoff that shows `endpoint_id` and
    /// holds no activation on its next turn.
    fn hand_off_to(&mut self, endpoint_id: &ClientEndpointId) {
        assert!(self.shell.activate_endpoint_projection(endpoint_id));
        self.shell.receive_endpoint_activations_held(false);
    }

    /// The terminal reports that its window gained or lost focus.
    fn window_focus(&mut self, focused: bool) {
        self.shell.handle_raw_events(vec![if focused {
            crate::raw_input::RawInputEvent::OuterFocusGained
        } else {
            crate::raw_input::RawInputEvent::OuterFocusLost
        }]);
    }

    /// The next snapshot of a machine, changed by `change`.
    fn update(
        &mut self,
        endpoint_id: &ClientEndpointId,
        change: impl FnOnce(&mut crate::protocol::ClientShellSnapshot),
    ) {
        let mut next = retained(&self.shell, endpoint_id);
        next.revision += 1;
        change(&mut next);
        self.shell
            .set_endpoint_snapshot(endpoint_id, Box::new(next));
    }
}

#[test]
fn the_first_turn_reports_to_the_local_daemon() {
    let mut client = Client::new();

    client.turn();

    let sent = requests(&client.local_sent);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "boot-1", "the lane request names Local's boot");
    let report = &reports(&client.local_sent)[0];
    assert_eq!(report.active_endpoint_id.as_deref(), Some("local"));
    assert_eq!(report.endpoints.len(), 2);
    assert_eq!(
        panes(report),
        [
            (client.remote.storage_key().as_str(), "pane_7"),
            ("local", "pane_2"),
            ("local", "pane_1"),
        ]
    );
    assert!(client.remote_sent.lock().unwrap().is_empty());
}

#[test]
fn a_turn_without_changes_sends_nothing() {
    let mut client = Client::new();
    client.turn();

    for _ in 0..5 {
        client.turn();
    }

    assert_eq!(requests(&client.local_sent).len(), 1);
}

#[test]
fn every_change_within_one_turn_becomes_one_report() {
    let mut client = Client::new();
    client.turn();

    // Three events arrive before the loop comes round again.
    let remote = client.remote.clone();
    client.update(&ClientEndpointId::Local, |snapshot| {
        snapshot.agents[1].title = Some("first".into());
    });
    client.update(&remote, |snapshot| {
        snapshot.agents[0].agent_status = AgentStatus::Working;
    });
    client.update(&ClientEndpointId::Local, |snapshot| {
        snapshot.agents[1].title = Some("second".into());
    });
    client.window_focus(true);
    client.turn();

    let reports = reports(&client.local_sent);
    assert_eq!(reports.len(), 2);
    let latest = &reports[1];
    assert_eq!(latest.focused, Some(true));
    assert_eq!(
        latest
            .agents
            .iter()
            .find(|row| row.agent.pane_id == "pane_2")
            .and_then(|row| row.agent.title.as_deref()),
        Some("second")
    );
    assert_eq!(
        latest
            .agents
            .iter()
            .find(|row| row.agent.pane_id == "pane_7")
            .map(|row| row.agent.agent_status),
        Some(AgentStatus::Working)
    );
}

#[test]
fn window_focus_changes_are_reported() {
    let mut client = Client::new();
    client.turn();
    assert_eq!(reports(&client.local_sent)[0].focused, None);

    client.window_focus(true);
    client.turn();
    client.window_focus(false);
    client.turn();
    client.turn();

    assert_eq!(
        reports(&client.local_sent)
            .iter()
            .map(|report| report.focused)
            .collect::<Vec<_>>(),
        [None, Some(true), Some(false)]
    );
}

#[test]
fn a_snapshot_that_changes_nothing_reported_is_not_sent_again() {
    let mut client = Client::new();
    client.turn();

    // A new revision whose agents, machines, and focus read the same.
    client.update(&ClientEndpointId::Local, |snapshot| {
        snapshot.tab_bar_right_separator = " | ".into();
    });
    client.turn();

    assert_eq!(requests(&client.local_sent).len(), 1);
}

#[test]
fn the_report_goes_to_local_while_an_ssh_endpoint_is_presented() {
    let mut client = Client::new();
    client.turn();
    let remote = client.remote.clone();

    // The user switches to Build: Local's surface goes quiet, its lane is
    // retired, and the registry's active connection is Build's.
    assert!(client.shell.activate_endpoint_projection(&remote));
    client.endpoints.set_surface_active(&remote, true);
    assert!(client.endpoints.set_active(&remote));
    client
        .endpoints
        .set_surface_active(&ClientEndpointId::Local, false);
    assert!(!client.endpoints.active_id().is_local());
    client.turn();

    // Build's agent changes while Build is presented.
    client.update(&remote, |snapshot| {
        snapshot.agents[0].agent_status = AgentStatus::Working;
    });
    client.turn();

    let reports = reports(&client.local_sent);
    assert_eq!(reports.len(), 3);
    assert_eq!(
        reports[1].active_endpoint_id,
        Some(remote.storage_key()),
        "the switch itself is reported"
    );
    assert!(reports[1]
        .agents
        .iter()
        .any(|row| row.focused && row.agent.pane_id == "pane_7"));
    assert_eq!(
        reports[2]
            .agents
            .iter()
            .find(|row| row.agent.pane_id == "pane_7")
            .map(|row| row.agent.agent_status),
        Some(AgentStatus::Working)
    );
    assert!(
        requests(&client.local_sent)
            .iter()
            .all(|(boot_id, _)| boot_id == "boot-1"),
        "every report names Local's boot, not the presented machine's"
    );
    assert!(
        client.remote_sent.lock().unwrap().is_empty(),
        "an SSH daemon is never reported to"
    );
}

#[test]
fn a_client_without_a_local_connection_reports_to_nobody() {
    let mut client = Client::new();
    client.endpoints.disconnect(&ClientEndpointId::Local);

    client.turn();
    client.window_focus(true);
    client.turn();

    assert!(client.local_sent.lock().unwrap().is_empty());
    assert!(client.remote_sent.lock().unwrap().is_empty());
}

#[test]
fn a_local_daemon_that_does_not_take_reports_gets_none() {
    let mut client = Client::new();
    let sent = Sent::default();
    client.endpoints.insert(
        ClientEndpointId::Local,
        Recorder(sent.clone()),
        3,
        EndpointNegotiation::new(vec!["pane.focus".into()], Vec::new()),
        true,
    );

    client.turn();

    assert!(sent.lock().unwrap().is_empty());
}

#[test]
fn nothing_is_reported_before_local_has_sent_a_snapshot() {
    let (mut shell, _) = fleet(&[]);
    let sent = Sent::default();
    let mut endpoints = EndpointRegistry::new(Recorder(sent.clone()), 1, negotiation());
    let mut contract = ClientContract::default();

    contract.sync(&mut shell, &mut endpoints, Instant::now());
    assert!(sent.lock().unwrap().is_empty(), "no boot id to name yet");

    shell.set_snapshot(Box::new(machine("boot-1", Vec::new())));
    contract.sync(&mut shell, &mut endpoints, Instant::now());
    assert_eq!(requests(&sent).len(), 1);
}

#[test]
fn a_new_local_connection_is_reported_to_afresh() {
    let mut client = Client::new();
    client.turn();

    // Local reconnects: the daemon dropped the report with the old connection.
    let reconnected = Sent::default();
    client.endpoints.insert(
        ClientEndpointId::Local,
        Recorder(reconnected.clone()),
        7,
        negotiation(),
        true,
    );
    client.turn();
    client.turn();

    assert_eq!(reports(&reconnected), reports(&client.local_sent));
}

#[test]
fn a_restarted_local_daemon_is_reported_to_under_its_new_boot() {
    let mut client = Client::new();
    client.turn();

    client.update(&ClientEndpointId::Local, |snapshot| {
        snapshot.boot_id = "boot-2".into();
    });
    client.turn();

    let sent = requests(&client.local_sent);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].0, "boot-2");
}

#[test]
fn request_ids_are_unique_and_kept_off_the_command_lane() {
    let mut client = Client::new();
    client.turn();
    client.window_focus(true);
    client.turn();

    let ids = requests(&client.local_sent)
        .into_iter()
        .map(|(_, request)| request.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|id| ClientContract::owns_response(id)));
    // The command lane's and the surface requests' ids are not this module's.
    assert!(!ClientContract::owns_response("client-shell:4"));
    assert!(!ClientContract::owns_response("client-shell-surface:4"));
}

#[test]
fn a_report_too_large_for_one_command_loses_its_last_rows() {
    let mut client = Client::new();
    let token = "t".repeat(crate::api::schema::MAX_CLIENT_REPORT_TEXT_BYTES);
    client.update(&ClientEndpointId::Local, |snapshot| {
        let template = snapshot.agents[0].clone();
        let pane = snapshot.panes[0].clone();
        snapshot.agents = (0..crate::api::schema::MAX_CLIENT_REPORT_AGENTS)
            .map(|index| crate::protocol::ClientShellAgent {
                pane_id: format!("pane_{index:04}"),
                focused: false,
                tokens: (0..8)
                    .map(|key| (format!("k{key}"), token.clone()))
                    .collect(),
                ..agent(&template.pane_id, AgentStatus::Done, index as u64 + 10)
            })
            .collect();
        snapshot.panes = snapshot
            .agents
            .iter()
            .map(|agent| crate::protocol::ClientShellPane {
                pane_id: agent.pane_id.clone(),
                focused: false,
                ..pane.clone()
            })
            .collect();
        snapshot.focused_pane_id = None;
    });
    let full = client.shell.client_report().expect("within the row limits");
    assert!(
        serde_json::to_string(&full).unwrap().len() > MAX_ENDPOINT_COMMAND_BYTES,
        "the limits alone do not bound a report to one command"
    );

    client.turn();

    let sent = client.local_sent.lock().unwrap();
    let [ClientMessage::ClientShellEndpointRequest { request, .. }] = sent.as_slice() else {
        panic!("one report: {}", sent.len());
    };
    assert!(request.len() <= MAX_ENDPOINT_COMMAND_BYTES);
    let request: Request = serde_json::from_str(request).unwrap();
    let Method::ClientReport(report) = request.method else {
        panic!("a report");
    };
    assert!(!report.agents.is_empty() && report.agents.len() < full.agents.len());
    // The rows kept are the head of the list, in order.
    assert_eq!(report.agents[..], full.agents[..report.agents.len()]);
    assert_eq!(report.endpoints, full.endpoints);
}

fn jump(endpoint_id: &str, boot_id: &str, pane_id: &str) -> EndpointClientJump {
    EndpointClientJump {
        jump_id: "jump-4".into(),
        endpoint_id: endpoint_id.into(),
        boot_id: boot_id.into(),
        pane_id: pane_id.into(),
    }
}

/// The jump answers a connection carried, as `(boot id, jump id, outcome)`.
fn answers(sent: &Sent) -> Vec<(String, String, ClientJumpOutcome)> {
    requests(sent)
        .into_iter()
        .filter_map(|(boot_id, request)| match request.method {
            Method::ClientJumpResolve(params) => {
                assert!(ClientContract::owns_response(&request.id), "{}", request.id);
                Some((boot_id, params.jump_id, params.outcome))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_jump_is_answered_on_the_connection_it_arrived_on() {
    let mut client = Client::new();
    client.turn();
    let origin = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };

    // Local's daemon forwards a jump to a pane on Build.
    let input = client.shell.receive_client_jump(
        origin,
        jump(&client.remote.storage_key(), "build-boot", "pane_7"),
        client.now,
    );
    assert_eq!(input.actions.len(), 1, "the navigation to dispatch");
    client.turn();
    assert!(answers(&client.local_sent).is_empty(), "still navigating");

    let remote = client.remote.clone();
    client.hand_off_to(&remote);
    client.turn();

    assert_eq!(
        answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "jump-4".to_owned(),
            ClientJumpOutcome::Focused
        )]
    );
    assert!(
        answers(&client.remote_sent).is_empty(),
        "Build's daemon never asked"
    );
    client.turn();
    assert_eq!(answers(&client.local_sent).len(), 1, "answered once");
}

#[test]
fn a_refused_jump_is_answered_in_the_same_turn() {
    let mut client = Client::new();
    let origin = ControlOrigin {
        endpoint_id: client.remote.clone(),
        generation: 2,
    };

    // Build's daemon forwards a jump whose boot id is out of date.
    let input =
        client
            .shell
            .receive_client_jump(origin, jump("local", "boot-0", "pane_2"), client.now);
    assert!(input.actions.is_empty());
    client.turn();

    assert_eq!(
        answers(&client.remote_sent),
        [(
            "build-boot".to_owned(),
            "jump-4".to_owned(),
            ClientJumpOutcome::StaleBoot
        )]
    );
    assert!(answers(&client.local_sent).is_empty());
}

#[test]
fn a_jump_that_does_not_settle_is_answered_with_a_timeout() {
    let mut client = Client::new();
    let origin = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    client.shell.receive_client_jump(
        origin,
        jump(&client.remote.storage_key(), "build-boot", "pane_7"),
        client.now,
    );

    client.now += NAVIGATION_SETTLE_DEADLINE - Duration::from_millis(1);
    client.turn();
    assert!(answers(&client.local_sent).is_empty());

    client.now += Duration::from_millis(1);
    client.turn();
    assert_eq!(
        answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "jump-4".to_owned(),
            ClientJumpOutcome::Timeout
        )]
    );
}

#[test]
fn an_answer_is_dropped_when_its_connection_is_gone() {
    let mut client = Client::new();
    client.turn();
    let origin = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    client
        .shell
        .receive_client_jump(origin, jump("local", "boot-0", "pane_2"), client.now);

    // Local reconnected before the turn: the old daemon connection, and the
    // jump it was waiting on, are gone.
    let reconnected = Sent::default();
    client.endpoints.insert(
        ClientEndpointId::Local,
        Recorder(reconnected.clone()),
        7,
        negotiation(),
        true,
    );
    client.turn();

    assert!(answers(&client.local_sent).is_empty());
    assert!(answers(&reconnected).is_empty());
}

#[test]
fn only_the_newest_jump_is_answered() {
    let mut client = Client::new();
    client.turn();
    let origin = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    client.shell.receive_client_jump(
        origin.clone(),
        jump(&client.remote.storage_key(), "build-boot", "pane_7"),
        client.now,
    );
    client.turn();

    // The daemon replaces that jump before it settled.
    let mut newer = jump("local", "boot-1", "pane_2");
    newer.jump_id = "jump-5".into();
    client.shell.receive_client_jump(origin, newer, client.now);
    client.update(&ClientEndpointId::Local, |snapshot| {
        snapshot.focused_pane_id = Some("pane_2".into());
    });
    client.shell.receive_endpoint_activations_held(false);
    client.turn();
    // Long past the first jump's deadline.
    client.now += NAVIGATION_SETTLE_DEADLINE * 2;
    client.turn();

    assert_eq!(
        answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "jump-5".to_owned(),
            ClientJumpOutcome::Focused
        )]
    );
}

#[test]
fn the_daemons_reply_to_an_answer_that_crossed_a_newer_jump_is_expected() {
    let mut client = Client::new();
    let origin = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    client
        .shell
        .receive_client_jump(origin, jump("local", "boot-0", "pane_2"), client.now);
    client.turn();
    let sent = requests(&client.local_sent);
    let answer_id = sent
        .iter()
        .find(|(_, request)| matches!(request.method, Method::ClientJumpResolve(_)))
        .map(|(_, request)| request.id.as_str())
        .expect("the jump's answer");
    let report_id = sent
        .iter()
        .find(|(_, request)| matches!(request.method, Method::ClientReport(_)))
        .map(|(_, request)| request.id.as_str())
        .expect("the report");

    // The daemon replaced the jump while the answer was on its way.
    assert!(answer_crossed_its_end(answer_id, Some("jump_not_found")));
    // Any other rejection of an answer, or of another request, is not.
    assert!(!answer_crossed_its_end(answer_id, Some("stale_boot")));
    assert!(!answer_crossed_its_end(answer_id, Some("pick_not_found")));
    assert!(!answer_crossed_its_end(answer_id, None));
    assert!(!answer_crossed_its_end(report_id, Some("jump_not_found")));
}

#[test]
fn the_daemons_reply_to_a_picker_answer_that_crossed_its_close_is_expected() {
    let mut client = Client::new();
    let local = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    assert!(client.shell.receive_pick_open(
        local,
        crate::protocol::endpoint::EndpointPickOpen {
            pick_id: "pick-1".into(),
            title: "Move pane".into(),
            items: Vec::new(),
            selected: None,
            create: None,
        },
    ));
    client.shell.handle_input_bytes(b"\x1b");
    client.turn();
    let sent = requests(&client.local_sent);
    let answer_id = sent
        .iter()
        .find(|(_, request)| matches!(request.method, Method::UiPickResolve(_)))
        .map(|(_, request)| request.id.as_str())
        .expect("the picker's answer");

    // The plugin closed the picker while the answer was on its way.
    assert!(answer_crossed_its_end(answer_id, Some("pick_not_found")));
    // A picker's answer is never rejected as a jump's.
    assert!(!answer_crossed_its_end(answer_id, Some("jump_not_found")));
    assert!(!answer_crossed_its_end(answer_id, Some("stale_boot")));
}

#[test]
fn a_picker_answer_is_measured_as_the_request_that_carries_it() {
    let created = |len: usize| crate::api::schema::UiPickOutcome::Created {
        text: "x".repeat(len),
    };
    let envelope = serde_json::to_string(&Request {
        id: format!("{PICK_REQUEST_PREFIX}{}", u64::MAX),
        method: Method::UiPickResolve(UiPickResolveParams {
            pick_id: "pick-1".into(),
            outcome: created(0),
        }),
    })
    .unwrap()
    .len();
    let room = MAX_ENDPOINT_COMMAND_BYTES - envelope;
    assert!(pick_answer_fits("pick-1", &created(room)));
    assert!(!pick_answer_fits("pick-1", &created(room + 1)));
    assert!(pick_answer_fits(
        "pick-1",
        &crate::api::schema::UiPickOutcome::Cancelled
    ));
}

/// The picker answers a connection carried, as `(boot id, pick id, outcome)`.
fn pick_answers(sent: &Sent) -> Vec<(String, String, crate::api::schema::UiPickOutcome)> {
    requests(sent)
        .into_iter()
        .filter_map(|(boot_id, request)| match request.method {
            Method::UiPickResolve(params) => {
                assert!(ClientContract::owns_response(&request.id), "{}", request.id);
                Some((boot_id, params.pick_id, params.outcome))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_jump_to_another_endpoint_cancels_the_open_picker_on_its_own_connection() {
    let mut client = Client::new();
    client.turn();
    let local = ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    };
    assert!(client.shell.receive_pick_open(
        local.clone(),
        crate::protocol::endpoint::EndpointPickOpen {
            pick_id: "pick-1".into(),
            title: "Move pane".into(),
            items: Vec::new(),
            selected: None,
            create: None,
        },
    ));
    client.turn();
    assert!(pick_answers(&client.local_sent).is_empty(), "still open");

    // A device jumps the client to Build while Local's plugin waits.
    client.shell.receive_client_jump(
        local,
        jump(&client.remote.storage_key(), "build-boot", "pane_7"),
        client.now,
    );
    let remote = client.remote.clone();
    client.hand_off_to(&remote);
    client.turn();

    // Local's daemon, which opened the picker, hears that it closed, though
    // the client now presents Build.
    assert_eq!(
        pick_answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "pick-1".to_owned(),
            crate::api::schema::UiPickOutcome::Cancelled
        )]
    );
    assert!(pick_answers(&client.remote_sent).is_empty());
    assert_eq!(
        answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "jump-4".to_owned(),
            ClientJumpOutcome::Focused
        )]
    );
    client.turn();
    assert_eq!(pick_answers(&client.local_sent).len(), 1, "answered once");
}
