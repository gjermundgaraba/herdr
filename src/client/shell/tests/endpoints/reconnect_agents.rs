//! The Agents list keeps a reconnecting machine's latest cached agents, and
//! selecting one routes only where the client can act on it.

use super::*;

fn state_with_reconnecting_agents(remote: bool) -> (ClientShellState, ClientEndpointId) {
    use crate::config::{AgentPanelSortConfig, AgentSidebarToken, StatusIndicatorStyle};
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.agent_panel_sort = AgentPanelSortConfig::Priority;
    state.config.status_indicators = StatusIndicatorStyle::Symbols;
    state.config.agents.rows = vec![vec![
        AgentSidebarToken::StateIcon.into(),
        AgentSidebarToken::Agent.into(),
    ]];
    let endpoint_id = if remote {
        let profile = remote_profile();
        let id = ClientEndpointId::Ssh(profile.id.clone());
        state.set_endpoint_catalog(&[profile]);
        state.set_endpoint_status(&id, ClientEndpointStatus::Online);
        id
    } else {
        ClientEndpointId::Local
    };
    let mut previous = snapshot();
    previous.agents = (1..=3)
        .map(|index| {
            let mut agent = agent(&format!("previous {index}"), AgentStatus::Blocked, index);
            agent.pane_id = format!("pane_{index}");
            agent.focused = index == 1;
            agent
        })
        .collect();
    let pane_template = previous.panes[0].clone();
    previous.panes = previous
        .agents
        .iter()
        .map(|agent| ClientShellPane {
            pane_id: agent.pane_id.clone(),
            focused: agent.focused,
            ..pane_template.clone()
        })
        .collect();
    state.cache_endpoint_snapshot_for_generation(&endpoint_id, 4, Box::new(previous));
    assert!(state.activate_endpoint_projection(&endpoint_id));
    state.set_pane_surface(surface());
    state.outer_focused = Some(false);
    state.mark_endpoint_disconnected(&endpoint_id);

    let mut replacement = snapshot();
    replacement.revision = 2;
    replacement.focused_pane_id = Some("pane_4".into());
    replacement.panes[0].pane_id = "pane_4".into();
    replacement.panes[0].label = None;
    let mut working = agent("latest working", AgentStatus::Working, 8);
    working.pane_id = "pane_4".into();
    let mut blocked = agent("latest blocked", AgentStatus::Blocked, 9);
    blocked.pane_id = "pane_2".into();
    blocked.focused = false;
    replacement.agents = vec![working, blocked];
    let pane_template = replacement.panes[0].clone();
    replacement.panes = replacement
        .agents
        .iter()
        .map(|agent| ClientShellPane {
            pane_id: agent.pane_id.clone(),
            focused: agent.focused,
            ..pane_template.clone()
        })
        .collect();
    state.cache_endpoint_snapshot_for_generation(&endpoint_id, 5, Box::new(replacement));
    (state, endpoint_id)
}

fn reconnect_agent_navigator() -> ClientNavigatorOverlay {
    ClientNavigatorOverlay {
        layout: NavigatorLayout::Agents,
        query: TextEditor::new("", false),
        search_focused: false,
        selected: None,
        scroll: 0,
        filter: None,
    }
}

#[test]
fn reconnect_agent_cache_updates_metadata_order_and_picker_focus() {
    use crate::config::AgentPanelSortConfig;

    let (state, _) = state_with_reconnecting_agents(true);
    let rows = aggregate_navigation::aggregate_agent_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        AgentPanelSortConfig::Priority,
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.agent.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["pane_2", "pane_4"]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.agent.agent_status)
            .collect::<Vec<_>>(),
        [AgentStatus::Blocked, AgentStatus::Working]
    );
    assert!(rows.iter().all(|row| row.endpoint.stale()));
    assert!(aggregate_navigation::online_agent_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        AgentPanelSortConfig::Priority,
    )
    .is_empty());
    let picker = aggregate_navigation::navigator_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        &reconnect_agent_navigator(),
    );
    assert_eq!(picker.len(), 2);
    assert!(picker[0].label.contains("latest blocked"));
    assert!(picker[1].label.contains("latest working"));
    assert!(picker.iter().all(|row| row.stale));
    assert!(!picker[0].current);
    assert!(picker[1].current);
}

#[test]
fn reconnect_agent_rendering_retains_pane_until_coherent_activation() {
    for remote in [false, true] {
        let (mut state, endpoint_id) = state_with_reconnecting_agents(remote);
        assert_eq!(state.snapshot.as_ref().unwrap().revision, 1);
        assert_eq!(state.pane_surface.as_ref().unwrap().projection_revision, 1);
        for collapsed in [false, true] {
            state.sidebar_collapsed = collapsed;
            let frame = state
                .compose(106, 28)
                .expect("retained pane with latest list");
            let buffer = frame.to_ratatui_buffer().unwrap();
            let text = frame
                .cells
                .iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>();
            assert!(text.contains("LIVE"), "retained pane: {text}");
            assert_eq!(
                state
                    .hits
                    .endpoint_agents
                    .iter()
                    .map(|(_, _, pane)| pane.as_str())
                    .collect::<Vec<_>>(),
                ["pane_2", "pane_4"]
            );
            let rect = state.hits.endpoint_agents[0].0;
            let icon = (rect.x..rect.right())
                .map(|x| &buffer[(x, rect.y)])
                .find(|cell| cell.symbol() == "×")
                .expect("cached blocked icon");
            assert_eq!(icon.fg, state.config.palette.overlay0);
            assert!(icon.modifier.contains(Modifier::DIM));
            if collapsed && endpoint_id.is_local() {
                let number = &buffer[(rect.x, rect.y)];
                assert_eq!(number.fg, state.config.palette.overlay0);
                assert!(number.modifier.contains(Modifier::DIM));
            }
            if !collapsed {
                assert!(text.contains("latest blocked"));
                assert!(text.contains("latest working"));
                assert!(!text.contains("previous"));
            }
        }
        state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
        assert!(state.activate_endpoint_projection(&endpoint_id));
        assert!(state.compose(106, 28).is_none());
        let mut replacement_surface = surface();
        replacement_surface.projection_revision = 2;
        replacement_surface.panes[0].pane_id = "pane_4".into();
        state.set_pane_surface(replacement_surface);
        assert!(state.compose(106, 28).is_some());
    }
}

fn assert_reconnect_agent_selection(outcome: &ClientShellInput, endpoint_id: &ClientEndpointId) {
    if endpoint_id.is_local() {
        assert!(matches!(
            outcome.actions.as_slice(),
            [ClientShellAction::ActivateEndpoint {
                endpoint_id: ClientEndpointId::Local,
                target: Some(ClientEndpointFocusTarget::Pane(pane)),
            }] if pane == "pane_2"
        ));
    } else {
        assert!(outcome.actions.is_empty());
    }
}

#[test]
fn reconnect_desktop_agent_selection_routes_local_and_rejects_remote() {
    for remote in [false, true] {
        let (mut state, endpoint_id) = state_with_reconnecting_agents(remote);
        state.compose(106, 28).unwrap();
        let rect = state.hits.endpoint_agents[0].0;
        let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })]);
        assert_reconnect_agent_selection(&outcome, &endpoint_id);
    }
}

#[test]
fn reconnect_picker_agent_selection_routes_local_and_rejects_remote() {
    for remote in [false, true] {
        let (mut state, endpoint_id) = state_with_reconnecting_agents(remote);
        let navigator = reconnect_agent_navigator();
        let rows = aggregate_navigation::navigator_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            &navigator,
        );
        state.overlay = Some(ClientShellOverlay::Navigator(ClientNavigatorOverlay {
            selected: Some(rows[0].target.clone()),
            ..navigator
        }));
        let mut outcome = ClientShellInput::default();
        state.accept_navigator_selection(&mut outcome);
        assert_reconnect_agent_selection(&outcome, &endpoint_id);
        assert_eq!(state.overlay.is_none(), endpoint_id.is_local());
    }
}

#[test]
fn reconnect_mobile_agent_selection_routes_local_and_rejects_remote() {
    for remote in [false, true] {
        let (mut state, endpoint_id) = state_with_reconnecting_agents(remote);
        state.mode = ClientShellMode::Navigate;
        state.compose(44, 40).expect("cached mobile agent list");
        let mobile_agents = state
            .hits
            .mobile_targets
            .iter()
            .filter_map(|(rect, target)| match target {
                ClientMobileTarget::Agent {
                    endpoint_id: selected,
                    pane_id,
                } => {
                    assert_eq!(selected, &endpoint_id);
                    Some((*rect, pane_id.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            mobile_agents
                .iter()
                .map(|(_, pane)| pane.as_str())
                .collect::<Vec<_>>(),
            ["pane_2", "pane_4"]
        );
        let rect = mobile_agents[0].0;
        let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::empty(),
        })]);
        assert_reconnect_agent_selection(&outcome, &endpoint_id);
        assert_eq!(
            state.mode,
            if remote {
                ClientShellMode::Navigate
            } else {
                ClientShellMode::Terminal
            }
        );
    }
}

#[test]
fn agent_navigation_uses_cached_focus_only_when_endpoint_is_online() {
    for remote in [false, true] {
        let (mut state, endpoint_id) = state_with_reconnecting_agents(remote);
        for action in [
            crate::input::KeybindAction::FocusAgent(0),
            crate::input::KeybindAction::NextAgent,
            crate::input::KeybindAction::PreviousAgent,
        ] {
            let mut outcome = ClientShellInput::default();
            assert!(state.handle_endpoint_navigation(action, &mut outcome));
            assert!(outcome.actions.is_empty());
        }
        state.set_endpoint_status(&endpoint_id, ClientEndpointStatus::Online);
        for (action, pane) in [
            (crate::input::KeybindAction::FocusAgent(1), "pane_4"),
            (crate::input::KeybindAction::NextAgent, "pane_2"),
            (crate::input::KeybindAction::PreviousAgent, "pane_2"),
        ] {
            let mut outcome = ClientShellInput::default();
            assert!(state.handle_endpoint_navigation(action, &mut outcome));
            assert!(matches!(outcome.actions.as_slice(),
                [ClientShellAction::Endpoint { endpoint_id: selected, request, .. }]
                if selected == &endpoint_id && matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == pane)));
        }
        assert_eq!(state.snapshot.as_ref().unwrap().revision, 1);
        assert_eq!(state.pane_surface.as_ref().unwrap().projection_revision, 1);
    }
}
