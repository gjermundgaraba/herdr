//! The `priority` order of the Agents list: daemons send each agent's
//! `priority_key`, and the client only compares keys and merges machines
//! (`aggregate_navigation::agent_order::priority_order`).

use super::fleet::{agent, fleet, keyed_agent, lifted_agent, listed, machine};
use super::*;
use crate::client::endpoint::ClientEndpointStatus;
use crate::config::AgentPanelSortConfig;

fn panes(state: &ClientShellState) -> Vec<String> {
    let panes = listed(state)
        .into_iter()
        .map(|(_, pane)| pane)
        .collect::<Vec<_>>();
    // One machine's sidebar and agent keys read that machine's own list.
    if !state.multi_endpoint_active() {
        let snapshot = state.snapshot.as_deref().expect("one machine");
        assert_eq!(
            agent_sidebar::ordered_agent_pane_ids(snapshot, state.config.agent_panel_sort),
            panes
        );
    }
    panes
}

fn rows<'a>(order: &'a [(&'a str, &'a str)]) -> Vec<(String, String)> {
    order
        .iter()
        .map(|(machine, pane)| ((*machine).to_owned(), (*pane).to_owned()))
        .collect()
}

#[test]
fn one_machine_lists_agents_by_key() {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            agent("idle-old", AgentStatus::Idle, 1),
            agent("working", AgentStatus::Working, 6),
            agent("done-new", AgentStatus::Done, 5),
            agent("idle-new", AgentStatus::Idle, 7),
            agent("blocked", AgentStatus::Blocked, 4),
            lifted_agent("lifted-done", AgentStatus::Done, 8),
            agent("done-old", AgentStatus::Done, 2),
            lifted_agent("lifted-idle", AgentStatus::Idle, 3),
        ],
    )));

    // Lifted waiting agents first; then blocked, done oldest first, working,
    // and idle newest first. A lifted idle agent keeps its normal place.
    let expected = [
        "lifted-done",
        "blocked",
        "done-old",
        "done-new",
        "working",
        "idle-new",
        "lifted-idle",
        "idle-old",
    ];
    assert_eq!(panes(&state), expected);

    // The sidebar draws that order, expanded and collapsed, and the agent
    // keys follow it.
    state.set_pane_surface(surface());
    for collapsed in [false, true] {
        state.sidebar_collapsed = collapsed;
        state.compose(146, 80).expect("one machine frame");
        assert_eq!(
            state
                .hits
                .agents
                .iter()
                .map(|(_, pane)| pane.as_str())
                .collect::<Vec<_>>(),
            expected,
            "collapsed={collapsed}"
        );
    }
    let mut outcome = ClientShellInput::default();
    state.record_binding(
        crate::input::KeybindMatch::Action(crate::input::KeybindAction::FocusAgent(1)),
        &mut outcome,
    );
    assert!(matches!(outcome.actions.as_slice(),
        [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method,
            crate::api::schema::Method::PaneFocus(target) if target.pane_id == "blocked")));
}

#[test]
fn keys_are_compared_as_bytes_without_being_read() {
    let (mut state, _) = fleet(&[]);
    // None of these is a key this build's daemon would send. The order is
    // plain byte order all the same: no field is parsed, no number is read.
    // Only the empty key is special: it is no key, and follows every key.
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            keyed_agent("lower-z", AgentStatus::Blocked, "z"),
            keyed_agent("ten", AgentStatus::Idle, "10"),
            keyed_agent("nine", AgentStatus::Idle, "9"),
            keyed_agent("upper-z", AgentStatus::Done, "Z"),
            keyed_agent("empty", AgentStatus::Blocked, ""),
            keyed_agent("non-ascii", AgentStatus::Idle, "é"),
        ],
    )));

    assert_eq!(
        panes(&state),
        ["ten", "nine", "upper-z", "lower-z", "non-ascii", "empty"]
    );
}

#[test]
fn a_machine_that_sends_no_keys_follows_every_keyed_agent() {
    let (mut state, remotes) = fleet(&["Old"]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            agent("local-idle", AgentStatus::Idle, 5),
            agent("local-done", AgentStatus::Done, 2),
        ],
    )));
    // A daemon without `priority_key` decodes to empty keys. Its agents keep
    // their own order, after every keyed agent, even idle ones.
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "old-boot",
            vec![
                keyed_agent("old-blocked", AgentStatus::Blocked, ""),
                keyed_agent("old-idle", AgentStatus::Idle, ""),
            ],
        )),
    );

    assert_eq!(
        panes(&state),
        ["local-done", "local-idle", "old-blocked", "old-idle"]
    );
}

#[test]
fn equal_keys_on_one_machine_keep_the_snapshot_order() {
    let (mut state, _) = fleet(&[]);
    // No ordering event since the daemon started: every stamp is 0.
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            agent("c", AgentStatus::Idle, 0),
            agent("a", AgentStatus::Idle, 0),
            agent("first-done", AgentStatus::Done, 0),
            agent("b", AgentStatus::Idle, 0),
            agent("second-done", AgentStatus::Done, 0),
        ],
    )));

    assert_eq!(panes(&state), ["first-done", "second-done", "c", "a", "b"]);
}

#[test]
fn machines_merge_by_key() {
    let (mut state, remotes) = fleet(&["Build", "Lab"]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            agent("local-done", AgentStatus::Done, 20),
            agent("local-idle", AgentStatus::Idle, 5),
        ],
    )));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![
                agent("build-idle", AgentStatus::Idle, 9),
                agent("build-done", AgentStatus::Done, 10),
                lifted_agent("build-lifted", AgentStatus::Blocked, 40),
            ],
        )),
    );
    state.set_endpoint_snapshot(
        &remotes[1],
        Box::new(machine(
            "lab-boot",
            vec![
                agent("lab-blocked", AgentStatus::Blocked, 30),
                agent("lab-done", AgentStatus::Done, 15),
            ],
        )),
    );

    // One queue over all three machines: the stamps interleave them.
    assert_eq!(
        listed(&state),
        rows(&[
            ("Build", "build-lifted"),
            ("Lab", "lab-blocked"),
            ("Build", "build-done"),
            ("Lab", "lab-done"),
            ("Local", "local-done"),
            ("Build", "build-idle"),
            ("Local", "local-idle"),
        ])
    );
}

#[test]
fn equal_keys_across_machines_keep_machine_order_then_snapshot_order() {
    let (mut state, remotes) = fleet(&["Build", "Lab"]);
    let tied = |prefix: &str| {
        vec![
            agent(&format!("{prefix}-1"), AgentStatus::Done, 7),
            agent(&format!("{prefix}-idle"), AgentStatus::Idle, 0),
            agent(&format!("{prefix}-2"), AgentStatus::Done, 7),
        ]
    };
    // Delivered in another order than the machines are listed in.
    state.set_endpoint_snapshot(&remotes[1], Box::new(machine("lab-boot", tied("lab"))));
    state.set_snapshot(Box::new(machine("boot-1", tied("local"))));
    state.set_endpoint_snapshot(&remotes[0], Box::new(machine("build-boot", tied("build"))));

    assert_eq!(
        listed(&state),
        rows(&[
            ("Local", "local-1"),
            ("Local", "local-2"),
            ("Build", "build-1"),
            ("Build", "build-2"),
            ("Lab", "lab-1"),
            ("Lab", "lab-2"),
            ("Local", "local-idle"),
            ("Build", "build-idle"),
            ("Lab", "lab-idle"),
        ])
    );
}

#[test]
fn a_reconnecting_machine_lists_last_whatever_its_keys_say() {
    let (mut state, remotes) = fleet(&["Build", "Lab"]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![agent("local-idle", AgentStatus::Idle, 1)],
    )));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![
                lifted_agent("build-lifted", AgentStatus::Blocked, 2),
                agent("build-idle", AgentStatus::Idle, 3),
            ],
        )),
    );
    state.set_endpoint_snapshot(
        &remotes[1],
        Box::new(machine(
            "lab-boot",
            vec![agent("lab-done", AgentStatus::Done, 4)],
        )),
    );
    state.set_endpoint_status(&remotes[0], ClientEndpointStatus::Reconnecting);

    // The client's own rule applies before any key: its retained rows follow
    // every online row, still ordered by key among themselves.
    assert_eq!(
        listed(&state),
        rows(&[
            ("Lab", "lab-done"),
            ("Local", "local-idle"),
            ("Build", "build-lifted"),
            ("Build", "build-idle"),
        ])
    );
    // Keyboard navigation skips them altogether.
    let online = aggregate_navigation::online_agent_targets(
        &state.endpoints,
        &state.active_endpoint_id,
        AgentPanelSortConfig::Priority,
    )
    .into_iter()
    .map(|target| target.pane_id)
    .collect::<Vec<_>>();
    assert_eq!(online, ["lab-done", "local-idle"]);
}

#[test]
fn the_spaces_sort_ignores_keys() {
    let (mut state, remotes) = fleet(&["Build"]);
    state.config.agent_panel_sort = AgentPanelSortConfig::Spaces;
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![
            agent("local-idle", AgentStatus::Idle, 1),
            agent("local-blocked", AgentStatus::Blocked, 2),
        ],
    )));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![lifted_agent("build-lifted", AgentStatus::Blocked, 3)],
        )),
    );

    assert_eq!(
        listed(&state),
        rows(&[
            ("Local", "local-idle"),
            ("Local", "local-blocked"),
            ("Build", "build-lifted"),
        ])
    );
}

#[test]
fn the_picker_shows_the_mark_and_shares_the_order() {
    let (mut state, remotes) = fleet(&["Build"]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![agent("local-done", AgentStatus::Done, 1)],
    )));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![lifted_agent("build-lifted", AgentStatus::Blocked, 9)],
        )),
    );
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
        ClientNavigatorTarget::Pane { pane_id, .. } => Some((pane_id, row.mark)),
        _ => None,
    })
    .collect::<Vec<_>>();

    assert_eq!(
        picker,
        [
            ("build-lifted".to_owned(), Some("★".to_owned())),
            ("local-done".to_owned(), None),
        ]
    );
}
