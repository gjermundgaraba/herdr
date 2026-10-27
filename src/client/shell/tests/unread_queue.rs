//! The unread hold from this client's side. The daemon owns the hold
//! (`app/unread_hold.rs`, `server/headless/unread_holds.rs`); the client sends
//! `agent.mark_unread` and shows what the following snapshots say. `Daemon`
//! below replays what a daemon sends at each step of a hold.

use super::fleet::{agent, profiles};
use super::*;
use crate::client::endpoint::ClientEndpointStatus;
use crate::config::AgentPanelSortConfig;
use crate::input::{KeybindAction, KeybindMatch};

/// One endpoint as its daemon reports it. `tab_1` holds `pane_1`, an idle
/// agent with the focus, beside a plain `sibling` split. `tab_2` holds the
/// other agents, whose completions nobody has looked at yet.
struct Daemon {
    endpoint_id: ClientEndpointId,
    snapshot: ClientShellSnapshot,
    /// The latest lifecycle sequence.
    clock: u64,
}

impl Daemon {
    fn new(endpoint_id: ClientEndpointId, boot_id: &str, count: usize) -> Self {
        let mut snapshot = snapshot();
        snapshot.boot_id = boot_id.into();
        snapshot.revision = 0;
        let mut background = snapshot.tabs[0].clone();
        background.tab_id = "tab_2".into();
        background.number = 2;
        background.focused = false;
        snapshot.tabs.push(background);
        let tab_of = |index: usize| if index == 1 { "tab_1" } else { "tab_2" };
        let template = snapshot.panes[0].clone();
        let pane = |pane_id: String, tab_id: &str| ClientShellPane {
            focused: pane_id == "pane_1",
            pane_id,
            tab_id: tab_id.into(),
            ..template.clone()
        };
        snapshot.panes = (1..=count)
            .map(|index| pane(format!("pane_{index}"), tab_of(index)))
            .chain([pane("sibling".into(), "tab_1")])
            .collect();
        snapshot.agents = (1..=count)
            .map(|index| ClientShellAgent {
                tab_id: tab_of(index).into(),
                focused: index == 1,
                ..agent(&format!("pane_{index}"), AgentStatus::Idle, 0)
            })
            .collect();
        let mut daemon = Self {
            endpoint_id,
            snapshot,
            clock: 0,
        };
        daemon.event("pane_1", AgentStatus::Idle);
        for index in 2..=count {
            daemon.event(&format!("pane_{index}"), AgentStatus::Done);
        }
        daemon
    }

    fn agent(&mut self, pane_id: &str) -> &mut ClientShellAgent {
        self.snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.pane_id == pane_id)
            .expect("agent")
    }

    /// A status change: a new sequence.
    fn event(&mut self, pane_id: &str, status: AgentStatus) {
        self.clock += 1;
        let clock = self.clock;
        let agent = self.agent(pane_id);
        agent.agent_status = status;
        agent.state_change_seq = clock;
    }

    /// The marking client stopped presenting the pane: the completion is back
    /// as a new ordering event.
    fn restore_hold(&mut self, pane_id: &str) {
        self.event(pane_id, AgentStatus::Done);
    }

    /// The client focuses a pane.
    fn focus(&mut self, pane_id: &str) {
        let tab_id = self
            .snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .expect("pane")
            .tab_id
            .clone();
        self.snapshot.focused_tab_id = Some(tab_id.clone());
        self.snapshot.focused_pane_id = Some(pane_id.into());
        for tab in &mut self.snapshot.tabs {
            tab.focused = tab.tab_id == tab_id;
        }
        for pane in &mut self.snapshot.panes {
            pane.focused = pane.pane_id == pane_id;
        }
        for agent in &mut self.snapshot.agents {
            agent.focused = agent.pane_id == pane_id;
        }
    }

    /// Delivers the next snapshot.
    fn send(&mut self, state: &mut ClientShellState) {
        self.snapshot.revision += 1;
        state.set_endpoint_snapshot(&self.endpoint_id, Box::new(self.snapshot.clone()));
    }

    /// Presents the focused tab's panes on the client's surface.
    fn present(&self, state: &mut ClientShellState) {
        let mut presented = surface();
        presented.boot_id = self.snapshot.boot_id.clone();
        presented.projection_revision = self.snapshot.revision;
        let template = presented.panes[0].clone();
        presented.panes = self
            .snapshot
            .panes
            .iter()
            .filter(|pane| Some(&pane.tab_id) == self.snapshot.focused_tab_id.as_ref())
            .map(|pane| PaneSurfacePane {
                pane_id: pane.pane_id.clone(),
                focused: pane.focused,
                ..template.clone()
            })
            .collect();
        state.set_pane_surface(presented);
    }
}

/// A client on the `priority` sort whose Local daemon reports `count` agents,
/// all presented, with `pane_1` focused.
fn queue_state(count: usize) -> (ClientShellState, Daemon) {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.agent_panel_sort = AgentPanelSortConfig::Priority;
    let mut state = ClientShellState::new(config);
    let mut daemon = Daemon::new(ClientEndpointId::Local, "boot-1", count);
    daemon.send(&mut state);
    daemon.present(&mut state);
    (state, daemon)
}

fn add_remote(state: &mut ClientShellState) -> (ClientEndpointId, Daemon) {
    let profile = profiles(&["Build"]).remove(0);
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    let mut daemon = Daemon::new(remote.clone(), "remote-boot", 1);
    daemon.send(state);
    (remote, daemon)
}

fn status(state: &ClientShellState, endpoint_id: &ClientEndpointId, pane_id: &str) -> AgentStatus {
    state
        .endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .and_then(|snapshot| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.pane_id == pane_id)
        })
        .expect("agent")
        .agent_status
}

fn order(state: &ClientShellState, sort: AgentPanelSortConfig) -> Vec<(ClientEndpointId, String)> {
    aggregate_navigation::aggregate_agent_rows(&state.endpoints, &state.active_endpoint_id, sort)
        .into_iter()
        .map(|row| (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone()))
        .collect()
}

fn local_order(state: &ClientShellState, sort: AgentPanelSortConfig) -> Vec<String> {
    order(state, sort)
        .into_iter()
        .filter(|(endpoint_id, _)| endpoint_id.is_local())
        .map(|(_, pane_id)| pane_id)
        .collect()
}

fn mark_unread(state: &mut ClientShellState) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        KeybindMatch::Action(KeybindAction::MarkUnread),
        &mut outcome,
    );
    outcome
}

/// The one request an input produced, with the endpoint lane it is for.
fn request(outcome: &ClientShellInput) -> (&ClientEndpointId, &crate::api::schema::Request) {
    match outcome.actions.as_slice() {
        [ClientShellAction::Endpoint {
            endpoint_id,
            request,
            ..
        }] => (endpoint_id, request),
        actions => panic!("expected one endpoint request: {actions:?}"),
    }
}

fn assert_marks(outcome: &ClientShellInput, endpoint: &ClientEndpointId, pane: &str) {
    let (endpoint_id, request) = request(outcome);
    assert_eq!(endpoint_id, endpoint);
    assert!(
        matches!(&request.method,
            crate::api::schema::Method::AgentMarkUnread(target) if target.pane_id == pane),
        "{request:?}"
    );
}

#[test]
fn mark_unread_asks_the_daemon_and_records_nothing_itself() {
    let (mut state, _daemon) = queue_state(3);
    state.set_endpoint_methods(Some(vec!["agent.mark_unread".into()]));
    let before = order(&state, AgentPanelSortConfig::Priority);

    let outcome = mark_unread(&mut state);

    assert_marks(&outcome, &ClientEndpointId::Local, "pane_1");
    // Nothing changes until the daemon says so.
    assert_eq!(
        status(&state, &ClientEndpointId::Local, "pane_1"),
        AgentStatus::Idle
    );
    assert_eq!(order(&state, AgentPanelSortConfig::Priority), before);
}

#[test]
fn mark_unread_goes_to_the_endpoint_that_owns_the_focused_pane() {
    let (mut state, _daemon) = queue_state(3);
    let (remote, remote_daemon) = add_remote(&mut state);
    assert!(state.activate_endpoint_projection(&remote));
    remote_daemon.present(&mut state);

    let outcome = mark_unread(&mut state);

    // Both endpoints have a `pane_1`; the mark is for the presented one.
    assert_marks(&outcome, &remote, "pane_1");
    let ClientShellAction::Endpoint { boot_id, .. } = &outcome.actions[0] else {
        unreachable!("checked above");
    };
    assert_eq!(boot_id, "remote-boot");
}

#[test]
fn mark_unread_without_a_focused_pane_sends_nothing() {
    let (mut state, mut daemon) = queue_state(3);
    daemon.snapshot.focused_pane_id = None;
    daemon.send(&mut state);

    let outcome = mark_unread(&mut state);

    assert!(outcome.actions.is_empty());
    assert_eq!(
        state.visible_endpoint_notice.as_ref().unwrap().title,
        "Nothing to mark"
    );
}

#[test]
fn a_server_without_the_method_disables_only_the_mark() {
    let (mut state, _daemon) = queue_state(3);
    state.set_endpoint_methods(Some(vec!["pane.focus".into()]));

    let outcome = mark_unread(&mut state);

    assert!(outcome.actions.is_empty());
    assert_eq!(
        state.visible_endpoint_notice.as_ref().unwrap().title,
        "Action unavailable"
    );
}

#[test]
fn the_daemons_answer_to_a_mark_is_shown() {
    let (mut state, _daemon) = queue_state(3);
    let outcome = mark_unread(&mut state);
    let request_id = request(&outcome).1.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    assert_eq!(
        state.visible_endpoint_notice.take().unwrap().title,
        "Marked unread"
    );

    let outcome = mark_unread(&mut state);
    let request_id = request(&outcome).1.id.clone();
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("agent_not_idle".into()),
            message: "only an acknowledged completion, shown as idle, can be marked unread".into(),
        }),
    );
    let notice = state.visible_endpoint_notice.take().unwrap();
    assert_eq!(notice.title, "Action rejected");
    assert!(notice.body.contains("acknowledged completion"));
}

#[test]
fn a_restored_hold_shows_its_badge_while_its_pane_is_only_in_view() {
    let (mut state, mut daemon) = queue_state(3);
    mark_unread(&mut state);

    // The user focuses the sibling split, so the daemon restores the hold.
    // The held pane never leaves the client's surface.
    daemon.focus("sibling");
    daemon.restore_hold("pane_1");
    daemon.send(&mut state);
    daemon.present(&mut state);

    assert_eq!(
        status(&state, &ClientEndpointId::Local, "pane_1"),
        AgentStatus::Done
    );
    // Its new sequence makes it the newest completion, so it leads them.
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_1", "pane_3", "pane_2"]
    );
}

fn restored_state(federated: bool) -> (ClientShellState, Vec<(ClientEndpointId, String)>) {
    let (mut state, mut daemon) = queue_state(3);
    let remote = federated.then(|| add_remote(&mut state).0);
    mark_unread(&mut state);
    daemon.focus("sibling");
    daemon.restore_hold("pane_1");
    daemon.send(&mut state);
    let mut expected = vec![
        (ClientEndpointId::Local, "pane_1".into()),
        (ClientEndpointId::Local, "pane_3".into()),
        (ClientEndpointId::Local, "pane_2".into()),
    ];
    if let Some(remote) = remote {
        expected.push((remote, "pane_1".into()));
    }
    (state, expected)
}

fn assert_pane_request(
    outcome: &ClientShellInput,
    endpoint: &ClientEndpointId,
    pane: &str,
    activation: bool,
) {
    if activation {
        assert!(matches!(outcome.actions.as_slice(),
            [ClientShellAction::ActivateEndpoint {
                endpoint_id, target: Some(ClientEndpointFocusTarget::Pane(target)),
            }] if endpoint_id == endpoint && target == pane));
    } else {
        assert!(matches!(outcome.actions.as_slice(),
            [ClientShellAction::Endpoint { endpoint_id, request, .. }]
            if endpoint_id == endpoint && matches!(&request.method,
                crate::api::schema::Method::PaneFocus(target) if target.pane_id == pane)));
    }
}

#[test]
fn the_restored_queue_is_shared_by_picker_and_keyboard_navigation() {
    for federated in [false, true] {
        let (mut state, expected) = restored_state(federated);
        assert_eq!(order(&state, AgentPanelSortConfig::Priority), expected);
        let navigator = ClientNavigatorOverlay {
            layout: NavigatorLayout::Agents,
            query: TextEditor::new("", false),
            search_focused: false,
            selected: None,
            scroll: 0,
            filter: None,
        };
        let picker = aggregate_navigation::navigator_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            &navigator,
        )
        .into_iter()
        .filter_map(|row| match row.target {
            ClientNavigatorTarget::Pane {
                endpoint_id,
                pane_id,
            } => Some((endpoint_id, pane_id)),
            _ => None,
        })
        .collect::<Vec<_>>();
        assert_eq!(picker, expected);
        let targets = aggregate_navigation::online_agent_targets(
            &state.endpoints,
            &state.active_endpoint_id,
            AgentPanelSortConfig::Priority,
        )
        .into_iter()
        .map(|target| (target.endpoint_id, target.pane_id))
        .collect::<Vec<_>>();
        assert_eq!(targets, expected);
        for (action, target) in [
            (KeybindAction::FocusAgent(2), &expected[2]),
            (KeybindAction::NextAgent, &expected[0]),
            (KeybindAction::PreviousAgent, expected.last().unwrap()),
        ] {
            let mut outcome = ClientShellInput::default();
            state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
            assert_pane_request(&outcome, &target.0, &target.1, federated);
        }
    }
}

#[test]
fn the_restored_queue_is_shared_by_desktop_and_mobile_surfaces() {
    for federated in [false, true] {
        let (mut state, expected) = restored_state(federated);
        let mut presented = surface();
        presented.projection_revision = state.snapshot.as_ref().unwrap().revision;
        state.set_pane_surface(presented);
        for collapsed in [false, true] {
            state.sidebar_collapsed = collapsed;
            state.compose(146, 40).unwrap();
            // One machine's list holds only its own panes.
            let rows = state
                .hits
                .agents
                .iter()
                .map(|(_, pane)| (ClientEndpointId::Local, pane.clone()))
                .chain(
                    state
                        .hits
                        .endpoint_agents
                        .iter()
                        .map(|(_, id, pane)| (id.clone(), pane.clone())),
                )
                .collect::<Vec<_>>();
            assert_eq!(rows, expected, "collapsed={collapsed}");
        }
        state.mode = ClientShellMode::Navigate;
        state.compose(44, 60).unwrap();
        let rows = state
            .hits
            .mobile_targets
            .iter()
            .filter_map(|(_, target)| match target {
                ClientMobileTarget::Agent {
                    endpoint_id,
                    pane_id,
                } => Some((endpoint_id.clone(), pane_id.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rows, expected);
    }
}

#[test]
#[ignore = "non-gating fixed-geometry unread queue composition profile"]
fn unread_queue_render_scale_profile() {
    for count in [1, 15] {
        for collapsed in [false, true] {
            for federated in [false, true] {
                for restored in [false, true] {
                    let (mut state, mut daemon) = queue_state(count);
                    state.sidebar_collapsed = collapsed;
                    if federated {
                        add_remote(&mut state);
                    }
                    if restored {
                        daemon.focus("sibling");
                        daemon.restore_hold("pane_1");
                        daemon.send(&mut state);
                    }
                    let mut next = surface();
                    next.projection_revision = state.snapshot.as_ref().unwrap().revision;
                    next.frame = FrameData::from_ratatui_buffer(
                        &Buffer::with_lines(vec!["x".repeat(120); 30]),
                        None,
                    );
                    let template = next.panes[0].clone();
                    next.panes = (0..count)
                        .map(|index| {
                            let mut pane = template.clone();
                            pane.pane_id = format!("pane_{}", index + 1);
                            pane.focused = false;
                            pane.inner_rect = SurfaceRect {
                                x: if count == 1 {
                                    0
                                } else {
                                    (index % 5) as u16 * 24
                                },
                                y: if count == 1 {
                                    0
                                } else {
                                    (index / 5) as u16 * 10
                                },
                                width: if count == 1 { 120 } else { 24 },
                                height: if count == 1 { 30 } else { 10 },
                            };
                            pane.rect = pane.inner_rect;
                            pane
                        })
                        .collect();
                    state.set_pane_surface(next);
                    assert_eq!(
                        state.snapshot.as_ref().unwrap().agents[0].agent_status,
                        if restored {
                            AgentStatus::Done
                        } else {
                            AgentStatus::Idle
                        }
                    );
                    state.compose(146, 32).unwrap();
                    let mut samples = Vec::new();
                    for _ in 0..200 {
                        let started = std::time::Instant::now();
                        std::hint::black_box(state.compose(146, 32).unwrap());
                        samples.push(started.elapsed().as_micros());
                    }
                    samples.sort_unstable();
                    eprintln!(
                        "unread-queue panes={count} collapsed={collapsed} federated={federated} restored={restored} median_us={} p95_us={}",
                        samples[100], samples[190]
                    );
                }
            }
        }
    }
}
