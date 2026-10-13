//! The fork's `priority` order: spaces with a `space_priority` token lift
//! their waiting agents, and waiting agents queue oldest change first.

use super::*;

#[test]
fn priority_token_lifts_done_agent_above_blocked_elsewhere() {
    let mut state = state_with_agents();
    state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
    let set_state = |state: &mut AppState, ws_idx: usize, agent_state, seen| {
        let pane_id = state.workspaces[ws_idx].tabs[0].root_pane;
        let pane = state.workspaces[ws_idx].tabs[0]
            .panes
            .get_mut(&pane_id)
            .unwrap();
        pane.seen = seen;
        let terminal_id = pane.attached_terminal_id.clone();
        state.terminals.get_mut(&terminal_id).unwrap().state = agent_state;
    };
    let set_token = |state: &mut AppState, key: &str, value: &str| {
        state.workspaces[0].metadata_tokens.patch(
            std::collections::HashMap::from([(key.to_string(), Some(value.to_string()))]),
            None,
            std::time::Instant::now(),
        );
    };
    set_state(&mut state, 0, AgentState::Idle, false);
    set_state(&mut state, 1, AgentState::Blocked, false);
    assert_eq!(projected_entries(&state)[0].ws_idx, 1);

    // Other tokens and an empty value lift nothing.
    set_token(&mut state, "lift", "★");
    set_token(&mut state, "space_priority", "");
    assert_eq!(projected_entries(&state)[0].ws_idx, 1);

    set_token(&mut state, "space_priority", "★");
    assert_eq!(projected_entries(&state)[0].ws_idx, 0);

    set_state(&mut state, 0, AgentState::Working, false);
    assert_eq!(projected_entries(&state)[0].ws_idx, 1);

    // Clearing the token drops the lift.
    set_state(&mut state, 0, AgentState::Idle, false);
    assert_eq!(projected_entries(&state)[0].ws_idx, 0);
    set_token(&mut state, "space_priority", "");
    assert_eq!(projected_entries(&state)[0].ws_idx, 1);
}

#[test]
fn priority_order_queues_waiting_agents_oldest_first() {
    // The order reads the stamps `priority_key` carries, not the sequences.
    let mut state = AppState::test_new();
    state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
    state.workspaces = vec![
        Workspace::test_new("new"),
        Workspace::test_new("unrecorded"),
        Workspace::test_new("old"),
    ];
    state.ensure_test_terminals();
    state.active = Some(0);
    for (ws_idx, stamp) in [(0, Some(7)), (1, None), (2, Some(3))] {
        let pane_id = state.workspaces[ws_idx].tabs[0].root_pane;
        let pane = state.workspaces[ws_idx].tabs[0]
            .panes
            .get_mut(&pane_id)
            .unwrap();
        pane.seen = false;
        let terminal_id = pane.attached_terminal_id.clone();
        let terminal = state.terminals.get_mut(&terminal_id).unwrap();
        terminal.detected_agent = Some(Agent::Claude);
        terminal.state = AgentState::Idle;
        terminal.agent_order_stamp = stamp;
        terminal.last_agent_state_change_seq = stamp.map(|stamp| 10 - stamp);
    }

    // No recorded change means none since the server started: oldest.
    let order = projected_entries(&state)
        .iter()
        .map(|entry| entry.ws_idx)
        .collect::<Vec<_>>();
    assert_eq!(order, [1, 2, 0]);
}
