//! One agent order across machines: online rows, the priority merge of
//! machines, and the priority mark the endpoint sends.

use super::*;

#[test]
fn keyboard_agent_navigation_reveals_online_rows_after_stale_spaces_rows() {
    for action in [
        crate::input::KeybindAction::FocusAgent(0),
        crate::input::KeybindAction::NextAgent,
    ] {
        let (mut state, remote) = state_with_scrollable_agents();
        state.config.agent_panel_sort = crate::config::AgentPanelSortConfig::Spaces;
        state.mark_endpoint_disconnected(&ClientEndpointId::Local);
        state.agent_scroll = 0;
        state.compose(100, 14).expect("stale spaces list");
        assert!(state
            .hits
            .endpoint_agents
            .iter()
            .all(|(_, endpoint, _)| endpoint.is_local()));
        let mut outcome = ClientShellInput::default();
        assert!(state.handle_endpoint_navigation(action, &mut outcome));
        assert!(matches!(outcome.actions.as_slice(),
            [ClientShellAction::ActivateEndpoint {
                endpoint_id, target: Some(ClientEndpointFocusTarget::Pane(pane)),
            }] if endpoint_id == &remote && pane == "pane_1"));
        // The target is revealed once the machine switch lands.
        assert!(state.activate_endpoint_projection(&remote));
        state.compose(100, 14).expect("online target revealed");
        assert!(state
            .hits
            .endpoint_agents
            .iter()
            .any(|(_, endpoint, pane)| endpoint == &remote && pane == "pane_1"));
    }
}

#[test]
fn mixed_legacy_fallback_priority_merges_agents_across_endpoints() {
    use crate::config::{AgentPanelSortConfig, AgentSidebarToken};

    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let remote = remote_profile();
    let remote_id = ClientEndpointId::Ssh(remote.id.clone());
    state.config.agent_panel_sort = AgentPanelSortConfig::Priority;
    state.config.agents.rows = vec![vec![
        AgentSidebarToken::Machine.into(),
        AgentSidebarToken::Agent.into(),
    ]];
    let mut third = remote_profile();
    third.id = ProfileId::parse("fedcba9876543210fedcba9876543210").unwrap();
    third.label = "Other".into();
    let third_id = ClientEndpointId::Ssh(third.id.clone());
    state.set_endpoint_catalog(&[remote, third]);
    state.set_endpoint_status(&remote_id, ClientEndpointStatus::Online);
    state.set_endpoint_status(&third_id, ClientEndpointStatus::Online);

    let mut local = snapshot();
    local.agent_view_label = Some("legacy".into());
    local.agent_order = vec!["pane_2".into(), "pane_1".into()];
    let mut second_idle = agent("local second idle", AgentStatus::Idle, 1);
    second_idle.pane_id = "pane_2".into();
    second_idle.focused = false;
    let mut excluded = agent("excluded blocked", AgentStatus::Blocked, 1);
    excluded.pane_id = "pane_3".into();
    excluded.focused = false;
    local.agents = vec![
        agent("local first idle", AgentStatus::Idle, 1),
        second_idle,
        excluded,
    ];
    let pane_template = local.panes[0].clone();
    local.panes = local
        .agents
        .iter()
        .map(|agent| ClientShellPane {
            pane_id: agent.pane_id.clone(),
            focused: agent.focused,
            ..pane_template.clone()
        })
        .collect();
    state.set_snapshot(Box::new(local));
    state.set_pane_surface(surface());
    let mut remote = snapshot();
    remote.boot_id = "remote-boot".into();
    remote.agents = vec![agent("remote blocked", AgentStatus::Blocked, 1)];
    state.set_endpoint_snapshot(&remote_id, Box::new(remote));
    let mut third = snapshot();
    third.boot_id = "third-boot".into();
    third.agents = vec![agent("other working", AgentStatus::Working, 1)];
    state.set_endpoint_snapshot(&third_id, Box::new(third));

    for rows in [
        aggregate_navigation::aggregate_agent_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            AgentPanelSortConfig::Priority,
        ),
        aggregate_navigation::online_agent_rows(
            &state.endpoints,
            &state.active_endpoint_id,
            AgentPanelSortConfig::Priority,
        ),
    ] {
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].endpoint.endpoint_id, &remote_id);
        assert_eq!(rows[1].endpoint.endpoint_id, &third_id);
        assert!(rows[2..]
            .iter()
            .all(|row| row.endpoint.endpoint_id.is_local()
                && row.agent.agent_status == AgentStatus::Idle));
    }
    let frame = state.compose(100, 28).expect("mixed legacy fallback frame");
    let text = frame
        .cells
        .chunks(frame.width as usize)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let blocked = text
        .find("Build · remote blocked")
        .expect("remote blocked row");
    let working = text
        .find("Other · other working")
        .expect("third endpoint row");
    assert!(
        blocked < working && working < text.find("Local · local").expect("local idle row"),
        "frame: {text}"
    );
    assert!(!text.contains("excluded blocked"));
    let mut outcome = ClientShellInput::default();
    assert!(
        state.handle_endpoint_navigation(crate::input::KeybindAction::FocusAgent(0), &mut outcome)
    );
    assert!(matches!(outcome.actions.as_slice(),
        [ClientShellAction::ActivateEndpoint { endpoint_id, target: Some(ClientEndpointFocusTarget::Pane(pane)) }]
        if endpoint_id == &remote_id && pane == "pane_1"));
}

#[test]
fn agent_picker_shows_the_priority_mark_the_endpoint_sent() {
    use crate::api::schema::AgentStatus;

    let mut snapshot = snapshot();
    let mut marked = snapshot.workspaces[0].clone();
    marked.workspace_id = "ws_2".into();
    marked.number = 2;
    marked.label = "urgent".into();
    marked.focused = false;
    // The space's own tokens are not consulted: the endpoint resolved the
    // mark from its `ui.agent_priority_tokens`.
    marked.tokens = vec![("urgent".into(), "!".into())];
    snapshot.workspaces.push(marked);
    let mut priority = agent("priority idle", AgentStatus::Idle, 2);
    priority.pane_id = "pane_2".into();
    priority.workspace_id = "ws_2".into();
    priority.focused = false;
    priority.priority_mark = Some("★".into());
    snapshot.agents = vec![agent("blocked", AgentStatus::Blocked, 1), priority];
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.config.theme_runtime.auto_switch = false;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface());
    state.open_navigator(NavigatorLayout::Agents);

    let Some(ClientShellOverlay::Navigator(navigator)) = state.overlay.as_ref() else {
        panic!("navigator");
    };
    let rows = aggregate_navigation::navigator_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        navigator,
    );
    assert_eq!(
        rows.iter()
            .map(|row| (row.label.as_str(), row.mark.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("client-shell: blocked", None),
            ("urgent: priority idle", Some("★"))
        ]
    );

    // The top row is selected, so the marked row keeps the mark's colour.
    let frame = state.compose(160, 48).expect("navigator");
    let (rect, _) = state
        .hits
        .navigator_rows
        .iter()
        .find(|(_, target)| {
            matches!(target, ClientNavigatorTarget::Pane { pane_id, .. } if pane_id == "pane_2")
        })
        .expect("marked row");
    let mark = cell_symbol_position(&frame, *rect, "★ urgent: priority idle");
    let buffer = frame.to_ratatui_buffer().expect("buffer");
    assert_eq!(buffer[mark].fg, state.config.palette.yellow);
    assert_eq!(buffer[(mark.0 + 2, mark.1)].fg, state.config.palette.text);
}
