use super::endpoints::CompletedSnapshots;
use super::*;
use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};
use crate::config::AgentPanelSortConfig;

fn agent(index: usize, status: AgentStatus, sequence: u64) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: format!("pane_{index}"),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some(format!("agent {index}")),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: sequence,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: index == 1,
    }
}

fn projection(count: usize) -> ClientShellSnapshot {
    let mut next = snapshot();
    next.agents = (1..=count)
        .map(|i| agent(i, AgentStatus::Idle, i as u64))
        .collect();
    let template = next.panes[0].clone();
    next.panes = next
        .agents
        .iter()
        .map(|a| ClientShellPane {
            pane_id: a.pane_id.clone(),
            focused: a.focused,
            ..template.clone()
        })
        .collect();
    next
}

fn queue_state(count: usize) -> ClientShellState {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.agent_panel_sort = AgentPanelSortConfig::Priority;
    let mut state = ClientShellState::new(config);
    state.set_snapshot_with_completions(Box::new(projection(count)));
    state.set_pane_surface(surface());
    let mut next = projection(count);
    next.revision += 1;
    for a in next.agents.iter_mut().skip(1) {
        a.agent_status = AgentStatus::Done;
        a.state_change_seq += count as u64;
    }
    state.set_snapshot_with_completions(Box::new(next));
    state
}

fn add_remote(state: &mut ClientShellState) -> ClientEndpointId {
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    let mut next = projection(1);
    next.boot_id = "remote-boot".into();
    state.set_endpoint_snapshot_with_completions(&remote, Box::new(next));
    remote
}

fn endpoint<'a>(state: &'a ClientShellState, id: &ClientEndpointId) -> &'a ClientShellEndpoint {
    state
        .endpoints
        .iter()
        .find(|e| &e.endpoint_id == id)
        .unwrap()
}

fn leave(state: &mut ClientShellState) {
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.focused_pane_id = None;
    for a in &mut next.agents {
        a.focused = false;
    }
    for p in &mut next.panes {
        p.focused = false;
    }
    state.set_snapshot_with_completions(Box::new(next));
}

fn mark_and_leave(state: &mut ClientShellState) {
    state.mark_focused_agent_unread(&mut ClientShellInput::default());
    assert!(state.pending_unread.is_some());
    leave(state);
}

fn local_order(state: &ClientShellState, sort: AgentPanelSortConfig) -> Vec<String> {
    aggregate_navigation::aggregate_agent_rows(&state.endpoints, &state.active_endpoint_id, sort)
        .into_iter()
        .filter(|row| row.endpoint.endpoint_id.is_local())
        .map(|row| row.agent.pane_id.clone())
        .collect()
}

fn aggregate_order(state: &ClientShellState) -> Vec<(ClientEndpointId, String)> {
    aggregate_navigation::aggregate_agent_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        AgentPanelSortConfig::Priority,
    )
    .into_iter()
    .map(|row| (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone()))
    .collect()
}

fn present(state: &mut ClientShellState, panes: &[&str]) {
    let snapshot = state.snapshot.as_deref().unwrap();
    let mut presented = surface();
    presented.boot_id = snapshot.boot_id.clone();
    presented.projection_revision = snapshot.revision;
    let template = presented.panes[0].clone();
    presented.panes = panes
        .iter()
        .map(|id| PaneSurfacePane {
            pane_id: (*id).into(),
            ..template.clone()
        })
        .collect();
    state.set_pane_surface(presented);
}

fn revisit(state: &mut ClientShellState) {
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.focused_pane_id = Some("pane_1".into());
    state.set_snapshot_with_completions(Box::new(next));
    present(state, &["pane_1"]);
}

#[test]
fn simultaneous_zero_sequence_agents_keep_snapshot_order() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut next = projection(3);
    for agent in &mut next.agents {
        agent.state_change_seq = 0;
    }
    next.agents.swap(0, 2);
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_2", "pane_1"]
    );
}

#[test]
fn later_zero_sequence_agent_is_fresher_than_startup_ties() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut next = projection(2);
    for agent in &mut next.agents {
        agent.state_change_seq = 0;
        agent.agent_status = AgentStatus::Working;
    }
    state.set_snapshot_with_completions(Box::new(next.clone()));
    next.revision += 1;
    next.agents.push(agent(3, AgentStatus::Working, 0));
    next.panes.push(projection(3).panes.pop().unwrap());
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_1", "pane_2"]
    );
}

#[test]
fn equal_changed_sequences_share_recency_without_collapsing_manual_requeue() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut next = projection(3);
    for agent in &mut next.agents {
        agent.state_change_seq = 0;
    }
    state.set_snapshot_with_completions(Box::new(next.clone()));
    state.set_pane_surface(surface());
    mark_and_leave(&mut state);
    next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    for agent in next.agents.iter_mut().skip(1) {
        agent.agent_status = AgentStatus::Done;
        agent.state_change_seq = 4;
    }
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_1", "pane_2", "pane_3"]
    );
    let mut unchanged = state.snapshot.as_deref().unwrap().clone();
    unchanged.revision += 1;
    state.set_snapshot_with_completions(Box::new(unchanged));
    present(&mut state, &["pane_1", "pane_2", "pane_3"]);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3", "pane_1"]
    );
}

#[test]
fn unchanged_zero_sequence_agents_keep_manual_recency_when_a_peer_arrives() {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut next = projection(2);
    for agent in &mut next.agents {
        agent.state_change_seq = 0;
    }
    state.set_snapshot_with_completions(Box::new(next));
    state.set_pane_surface(surface());
    mark_and_leave(&mut state);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agents.push(agent(3, AgentStatus::Done, 0));
    next.panes.push(projection(3).panes.pop().unwrap());
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_1", "pane_3", "pane_2"]
    );
    present(&mut state, &["pane_1", "pane_2", "pane_3"]);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_1", "pane_2"]
    );
}

#[test]
fn mark_unread_uses_presented_focus_and_exact_sequence_when_cache_is_ahead() {
    for lifecycle_changed in [false, true] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot_with_completions(Box::new(projection(2)));
        state.set_pane_surface(surface());
        let mut cached = state.snapshot.as_deref().unwrap().clone();
        cached.revision += 1;
        cached.focused_pane_id = Some("pane_2".into());
        cached.agents[0].focused = false;
        cached.agents[1].focused = true;
        cached.panes[0].focused = false;
        cached.panes[1].focused = true;
        if lifecycle_changed {
            cached.agents[0].state_change_seq += 10;
            cached.agents[0].agent_status = AgentStatus::Working;
        }
        state.cache_endpoint_snapshot_inactive_for_generation(
            &ClientEndpointId::Local,
            1,
            Box::new(cached),
        );
        assert_eq!(
            state.snapshot.as_ref().unwrap().focused_pane_id.as_deref(),
            Some("pane_1")
        );
        state.mark_focused_agent_unread(&mut ClientShellInput::default());
        let held = state.pending_unread.as_ref().unwrap();
        assert_eq!(held.pane_id, "pane_1");
        assert_eq!(held.sequence, 1);
        assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
        assert!(state.pending_unread.is_none());
        let agents = &endpoint(&state, &ClientEndpointId::Local)
            .snapshot
            .as_ref()
            .unwrap()
            .agents;
        assert_eq!(
            agents[0].agent_status,
            if lifecycle_changed {
                AgentStatus::Working
            } else {
                AgentStatus::Done
            }
        );
        // The cache's focused pane was never the target of the mark.
        assert_eq!(agents[1].agent_status, AgentStatus::Idle);
        assert_eq!(
            agents[0].state_change_seq,
            if lifecycle_changed { 11 } else { 1 }
        );
    }
}

#[test]
fn unread_requeues_on_departure_without_changing_lifecycle_sequence() {
    let mut state = queue_state(3);
    state.mark_focused_agent_unread(&mut ClientShellInput::default());
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Idle
    );
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3", "pane_1"]
    );
    leave(&mut state);
    let agent = &state.snapshot.as_ref().unwrap().agents[0];
    assert_eq!(agent.state_change_seq, 1);
    assert_eq!(agent.agent_status, AgentStatus::Done);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3", "pane_1"]
    );
}

#[test]
fn revisit_clears_badge_and_keeps_the_newer_idle_recency() {
    let mut state = queue_state(3);
    mark_and_leave(&mut state);
    revisit(&mut state);
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Idle
    );
    present(&mut state, &["pane_1", "pane_2", "pane_3"]);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_1", "pane_3", "pane_2"]
    );
}

#[test]
fn repeated_unread_reset_moves_behind_a_newer_completion() {
    let mut state = queue_state(3);
    mark_and_leave(&mut state);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agents[1].state_change_seq = 7;
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_1", "pane_2"]
    );
    revisit(&mut state);
    mark_and_leave(&mut state);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_2", "pane_1"]
    );
}

#[test]
fn real_lifecycle_change_cancels_pending_unread() {
    let mut state = queue_state(3);
    state.mark_focused_agent_unread(&mut ClientShellInput::default());
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agents[0].state_change_seq += 1;
    next.agents[0].agent_status = AgentStatus::Working;
    state.set_snapshot_with_completions(Box::new(next));
    leave(&mut state);
    assert!(state.pending_unread.is_none());
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Working
    );
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3", "pane_1"]
    );
}

#[test]
fn removing_the_pending_agent_does_not_restore_its_badge() {
    let mut state = queue_state(3);
    state.mark_focused_agent_unread(&mut ClientShellInput::default());
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.focused_pane_id = None;
    next.agents.remove(0);
    next.panes.remove(0);
    state.set_snapshot_with_completions(Box::new(next));
    assert!(state.pending_unread.is_none());
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3"]
    );
}

#[test]
fn new_boot_reseeds_order_and_removed_agents_leave_the_queue() {
    let mut state = queue_state(3);
    mark_and_leave(&mut state);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.boot_id = "boot-2".into();
    next.revision += 1;
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_2", "pane_1"]
    );
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agents.remove(0);
    next.panes.remove(0);
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_2"]
    );
}

#[test]
fn remote_unread_order_survives_same_boot_reconnect_and_endpoint_removal() {
    let mut state = queue_state(3);
    let remote = add_remote(&mut state);
    assert!(state.activate_endpoint_projection(&remote));
    present(&mut state, &["pane_1"]);
    state.mark_focused_agent_unread(&mut ClientShellInput::default());
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    let expected = vec![
        (ClientEndpointId::Local, "pane_2".into()),
        (ClientEndpointId::Local, "pane_3".into()),
        (remote.clone(), "pane_1".into()),
        (ClientEndpointId::Local, "pane_1".into()),
    ];
    assert_eq!(aggregate_order(&state), expected);
    let next = endpoint(&state, &remote).snapshot.clone().unwrap();
    state.mark_endpoint_disconnected(&remote);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    state.set_endpoint_snapshot_for_generation(&remote, 99, next);
    assert_eq!(aggregate_order(&state), expected);
    state.set_endpoint_catalog(&[]);
    assert_eq!(
        aggregate_order(&state)
            .into_iter()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>(),
        ["pane_2", "pane_3", "pane_1"]
    );
}

#[test]
fn pane_identity_change_reages_builtin_local_and_aggregate_order() {
    let mut state = queue_state(3);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agents[1].pane_id = "replacement".into();
    next.panes[1].pane_id = "replacement".into();
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "replacement", "pane_1"]
    );
    assert_eq!(
        aggregate_order(&state)
            .into_iter()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>(),
        ["pane_3", "replacement", "pane_1"]
    );
}

#[test]
fn reset_preserves_spaces_order() {
    let mut state = queue_state(3);
    mark_and_leave(&mut state);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Spaces),
        ["pane_1", "pane_2", "pane_3"]
    );
}

#[test]
fn legacy_custom_order_preserves_membership_and_yields_to_builtin_priority() {
    let mut state = queue_state(3);
    mark_and_leave(&mut state);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.agent_view_label = Some("custom".into());
    next.agent_order = vec!["pane_1".into(), "pane_3".into(), "pane_2".into()];
    next.agents.reverse();
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Spaces),
        ["pane_1", "pane_3", "pane_2"]
    );
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_3", "pane_1"]
    );

    let mut next = state.snapshot.as_deref().unwrap().clone();
    next.revision += 1;
    next.agent_order.pop();
    state.set_snapshot_with_completions(Box::new(next));
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Spaces),
        ["pane_1", "pane_3"]
    );
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_3", "pane_1"]
    );
}

#[test]
fn reset_stays_within_its_priority_space_and_status_group() {
    let mut state = queue_state(3);
    let mut next = state.snapshot.as_deref().unwrap().clone();
    let mut marked = next.workspaces[0].clone();
    marked.workspace_id = "priority".into();
    marked.tokens = vec![(crate::space_priority::TOKEN.into(), "★".into())];
    next.workspaces.push(marked);
    next.agents[0].workspace_id = "priority".into();
    next.agents[1].agent_status = AgentStatus::Blocked;
    next.agents[1].workspace_id = "priority".into();
    next.revision += 1;
    state.set_snapshot_with_completions(Box::new(next));
    mark_and_leave(&mut state);
    assert_eq!(
        local_order(&state, AgentPanelSortConfig::Priority),
        ["pane_2", "pane_1", "pane_3"]
    );
    assert_eq!(
        aggregate_order(&state)
            .into_iter()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>(),
        ["pane_2", "pane_1", "pane_3"]
    );
}

fn reset_surface_state(federated: bool) -> (ClientShellState, Vec<(ClientEndpointId, String)>) {
    let mut state = queue_state(3);
    let remote = federated.then(|| add_remote(&mut state));
    mark_and_leave(&mut state);
    let mut expected = vec![
        (ClientEndpointId::Local, "pane_2".into()),
        (ClientEndpointId::Local, "pane_3".into()),
        (ClientEndpointId::Local, "pane_1".into()),
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
fn reset_order_is_shared_by_picker_and_keyboard_navigation() {
    for federated in [false, true] {
        let (mut state, expected) = reset_surface_state(federated);
        assert_eq!(aggregate_order(&state), expected);
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
        let targets = aggregate_navigation::online_agent_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            AgentPanelSortConfig::Priority,
        )
        .into_iter()
        .map(|row| (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone()))
        .collect::<Vec<_>>();
        assert_eq!(targets, expected);
        for (action, target) in [
            (crate::input::KeybindAction::FocusAgent(2), &expected[2]),
            (crate::input::KeybindAction::NextAgent, &expected[0]),
            (
                crate::input::KeybindAction::PreviousAgent,
                expected.last().unwrap(),
            ),
        ] {
            let mut outcome = ClientShellInput::default();
            assert!(state.handle_endpoint_navigation(action, &mut outcome));
            assert_pane_request(&outcome, &target.0, &target.1, federated);
        }
    }
}

#[test]
fn reset_order_is_shared_by_desktop_and_mobile_surfaces() {
    for federated in [false, true] {
        let (mut state, expected) = reset_surface_state(federated);
        state.outer_focused = Some(false);
        let mut presented = surface();
        presented.projection_revision = state.snapshot.as_ref().unwrap().revision;
        state.set_pane_surface(presented);
        for collapsed in [false, true] {
            state.sidebar_collapsed = collapsed;
            state.compose(146, 40).unwrap();
            let rows = state
                .hits
                .endpoint_agents
                .iter()
                .map(|(_, id, pane)| (id.clone(), pane.clone()))
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
                for reset in [false, true] {
                    let mut state = queue_state(count);
                    state.sidebar_collapsed = collapsed;
                    if federated {
                        add_remote(&mut state);
                    }
                    if reset {
                        mark_and_leave(&mut state);
                    }
                    // Compare queue ordering after the confirmation toast expires;
                    // its occlusion work otherwise dominates this measurement.
                    state.visible_endpoint_notice = None;
                    state.outer_focused = Some(false);
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
                        if reset {
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
                        "unread-queue panes={count} collapsed={collapsed} federated={federated} reset={reset} median_us={} p95_us={}",
                        samples[100], samples[190]
                    );
                }
            }
        }
    }
}
