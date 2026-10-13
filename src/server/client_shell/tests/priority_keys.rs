//! The `priority_key` and `priority_mark` the fork's snapshot sends per agent.

use super::*;

/// One agent per named space, each in the given state and seen flag.
pub(super) fn app_with_agents(
    agents: &[(&str, crate::detect::AgentState, bool)],
) -> crate::app::App {
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = crate::app::App::new(
        &crate::config::Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        api_rx,
        crate::api::EventHub::default(),
    );
    app.state.workspaces = agents
        .iter()
        .map(|(name, _, _)| crate::workspace::Workspace::test_new(name))
        .collect();
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    for (ws_idx, &(_, state, seen)) in agents.iter().enumerate() {
        let pane_id = app.state.workspaces[ws_idx].tabs[0].root_pane;
        let pane = app.state.workspaces[ws_idx]
            .pane_state_mut(pane_id)
            .unwrap();
        pane.seen = seen;
        let terminal_id = pane.attached_terminal_id.clone();
        let (sequence, stamp) = app.state.next_agent_order_event();
        let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
        terminal.detected_agent = Some(crate::detect::Agent::Claude);
        terminal.state = state;
        terminal.last_agent_state_change_seq = Some(sequence);
        terminal.agent_order_stamp = Some(stamp);
    }
    app
}

pub(super) fn set_token(app: &mut crate::app::App, ws_idx: usize, key: &str, value: &str) {
    app.state.workspaces[ws_idx].metadata_tokens.patch(
        std::collections::HashMap::from([(key.to_owned(), Some(value.to_owned()))]),
        None,
        std::time::Instant::now(),
    );
}

/// Pane ids in the order a client gets by sorting on `priority_key`,
/// ties keeping snapshot order.
fn key_order(snapshot: &protocol::ClientShellSnapshot) -> Vec<String> {
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    agents.sort_by(|left, right| left.priority_key.cmp(&right.priority_key));
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

#[test]
fn snapshot_priority_keys_reproduce_the_priority_order_with_the_lift() {
    use crate::detect::AgentState::{Blocked, Idle, Working};
    let mut app = app_with_agents(&[
        ("idle-old", Idle, true),
        ("done-old", Idle, false),
        ("working", Working, true),
        ("blocked", Blocked, true),
        ("done-new", Idle, false),
        ("idle-new", Idle, true),
    ]);
    app.state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
    let pane = |snapshot: &protocol::ClientShellSnapshot, index: usize| {
        snapshot
            .agents
            .iter()
            .find(|agent| agent.workspace_id == snapshot.workspaces[index].workspace_id)
            .unwrap()
            .pane_id
            .clone()
    };

    let plain = snapshot(&app, "boot", 1, None, None);
    // Blocked, then done oldest first, then working, then idle newest first.
    let expected = [3, 1, 4, 2, 5, 0].map(|index| pane(&plain, index));
    assert_eq!(key_order(&plain), expected);
    assert_eq!(plain.agent_order, expected);
    assert!(plain
        .agents
        .iter()
        .all(|agent| agent.priority_mark.is_none() && agent.priority_key.starts_with("1.")));

    // Only a non-empty `space_priority` token lifts and marks.
    set_token(&mut app, 4, "space_priority", "★");
    set_token(&mut app, 2, "space_priority", "now");
    set_token(&mut app, 0, "space_priority", "");
    set_token(&mut app, 3, "lift", "x");
    let lifted = snapshot(&app, "boot", 2, None, None);
    // The lifted done agent passes the blocked one; the lifted working
    // agent keeps its place, and so does the space with an empty token.
    let expected = [4, 3, 1, 2, 5, 0].map(|index| pane(&lifted, index));
    assert_eq!(key_order(&lifted), expected);
    assert_eq!(lifted.agent_order, expected);
    let marks = lifted
        .workspaces
        .iter()
        .map(|workspace| {
            lifted
                .agents
                .iter()
                .find(|agent| agent.workspace_id == workspace.workspace_id)
                .unwrap()
                .priority_mark
                .as_deref()
        })
        .collect::<Vec<_>>();
    assert_eq!(marks, [None, None, Some("now"), None, Some("★"), None]);
}

#[test]
fn snapshot_priority_keys_requeue_a_restored_completion() {
    use crate::detect::AgentState::Idle;
    let mut app = app_with_agents(&[
        ("held", Idle, true),
        ("done-first", Idle, false),
        ("done-second", Idle, false),
    ]);
    let order = |app: &crate::app::App| {
        let snapshot = snapshot(app, "boot", 1, None, None);
        key_order(&snapshot)
            .into_iter()
            .map(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id)
                    .map(|agent| {
                        snapshot
                            .workspaces
                            .iter()
                            .position(|workspace| workspace.workspace_id == agent.workspace_id)
                            .unwrap()
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&app), [1, 2, 0]);

    let held = app.state.workspaces[0].tabs[0].root_pane;
    app.state.arm_unread_hold(0, held, None).unwrap();
    assert_eq!(order(&app), [1, 2, 0]);
    // Although it changed first, it now waits behind both completions.
    assert!(app.restore_unread_hold(0, held));
    assert_eq!(order(&app), [1, 2, 0]);
    let restored = snapshot(&app, "boot", 2, None, None);
    assert_eq!(
        restored.agents[0].agent_status,
        crate::api::schema::AgentStatus::Done
    );

    // Seen again, it is the freshest idle agent.
    assert!(app.state.clear_unread_hold(0, held));
    let first_done = app.state.workspaces[1].tabs[0].root_pane;
    app.state.workspaces[1]
        .pane_state_mut(first_done)
        .unwrap()
        .seen = true;
    assert_eq!(order(&app), [2, 0, 1]);
}
