use super::*;

fn endpoint() -> ClientEndpointId {
    ClientEndpointId::Ssh(
        super::super::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
    )
}

fn lease(id: ClientEndpointId, generation: u64, boot: &str) -> EndpointLease {
    EndpointLease {
        endpoint_id: id,
        generation,
        boot_id: boot.into(),
        minimum_revision: 0,
    }
}

#[derive(Clone)]
struct FakeTransport {
    sent: std::sync::Arc<std::sync::Mutex<Vec<crate::protocol::ClientMessage>>>,
    fail_after_write: bool,
}

impl super::super::EndpointTransport for FakeTransport {
    fn send(&mut self, message: &crate::protocol::ClientMessage) -> std::io::Result<()> {
        self.sent.lock().unwrap().push(message.clone());
        if self.fail_after_write {
            Err(std::io::Error::other("simulated observed write failure"))
        } else {
            Ok(())
        }
    }
}

fn negotiation() -> super::super::EndpointNegotiation {
    super::super::EndpointNegotiation::new(
        vec!["client_shell.surface.set".into()],
        vec![
            crate::protocol::endpoint::SURFACE_INTEREST_CAPABILITY.into(),
            crate::protocol::endpoint::PRESENTATION_EFFECTS_FENCE_CAPABILITY.into(),
        ],
    )
}

fn test_snapshot(boot_id: &str, revision: u64) -> crate::protocol::ClientShellSnapshot {
    crate::protocol::ClientShellSnapshot {
        boot_id: boot_id.into(),
        revision,
        config_diagnostic: None,
        product_announcement: None,
        update_available: None,
        update_install_command: String::new(),
        server_keybindings_toml: None,
        latest_release_notes_available: false,
        integration_updates_available: false,
        worktree_directory: String::new(),
        release_notes: None,
        focused_workspace_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        tab_bar_right: Vec::new(),
        tab_bar_right_separator: String::new(),
        agent_view_label: None,
        agent_order: Vec::new(),
        workspaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        agents: Vec::new(),
        commands: Vec::new(),
        pinned_sections: Vec::new(),
    }
}

type SentMessages = std::sync::Arc<std::sync::Mutex<Vec<crate::protocol::ClientMessage>>>;
type TestFixture = (
    crate::client::ClientShellState,
    EndpointRegistry,
    SentMessages,
    SentMessages,
);

fn shell_and_registry() -> TestFixture {
    shell_and_registry_with_source_failure(false)
}

fn shell_and_registry_with_source_failure(source_fail_after_write: bool) -> TestFixture {
    let mut shell = crate::client::ClientShellState::new(
        crate::client::ClientShellConfig::from_config(&crate::config::Config::default()),
    );
    let profile = super::super::SavedSshEndpoint {
        id: super::super::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Remote".into(),
        target: "dev@example.com".into(),
        session: "main".into(),
        enabled: true,
    };
    let target = ClientEndpointId::Ssh(profile.id.clone());
    shell.set_endpoint_catalog(&[profile]);
    shell.set_snapshot(Box::new(test_snapshot("local-boot", 1)));
    shell.set_endpoint_status(&target, ClientEndpointStatus::Online);
    shell.set_endpoint_snapshot(&target, Box::new(test_snapshot("remote-boot", 1)));

    let local_sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let remote_sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut endpoints = EndpointRegistry::new(
        FakeTransport {
            sent: local_sent.clone(),
            fail_after_write: source_fail_after_write,
        },
        1,
        negotiation(),
    );
    endpoints.insert(
        target,
        FakeTransport {
            sent: remote_sent.clone(),
            fail_after_write: false,
        },
        7,
        negotiation(),
        false,
    );
    (shell, endpoints, local_sent, remote_sent)
}

fn surface_success(id: &str, active: bool, projection_revision: u64) -> Vec<u8> {
    serde_json::to_vec(&crate::api::schema::SuccessResponse {
        id: id.into(),
        result: crate::api::schema::ResponseResult::ClientShellSurfaceSet {
            active,
            projection_revision,
        },
    })
    .unwrap()
}

fn workspace_focus_success(id: &str, workspace_id: &str) -> Vec<u8> {
    serde_json::to_vec(&crate::api::schema::SuccessResponse {
        id: id.into(),
        result: crate::api::schema::ResponseResult::WorkspaceInfo {
            workspace: crate::api::schema::WorkspaceInfo {
                workspace_id: workspace_id.into(),
                number: 1,
                label: workspace_id.into(),
                focused: true,
                pane_count: 1,
                tab_count: 1,
                active_tab_id: "tab".into(),
                agent_status: crate::api::schema::AgentStatus::Unknown,
                tokens: Default::default(),
                worktree: None,
                group: None,
                family_anchor_id: None,
            },
        },
    })
    .unwrap()
}

fn pane_focus_success(id: &str, pane_id: &str) -> Vec<u8> {
    serde_json::to_vec(&crate::api::schema::SuccessResponse {
        id: id.into(),
        result: crate::api::schema::ResponseResult::PaneInfo {
            pane: crate::api::schema::PaneInfo {
                pane_id: pane_id.into(),
                terminal_id: "terminal".into(),
                workspace_id: "workspace".into(),
                tab_id: "tab".into(),
                focused: true,
                cwd: None,
                foreground_cwd: None,
                restore_error: None,
                label: None,
                agent: None,
                title: None,
                terminal_title: None,
                terminal_title_stripped: None,
                display_agent: None,
                agent_status: crate::api::schema::AgentStatus::Unknown,
                state_labels: Default::default(),
                tokens: Default::default(),
                agent_session: None,
                scroll: None,
                revision: 0,
            },
        },
    })
    .unwrap()
}

fn failure(id: &str, message: &str) -> Vec<u8> {
    serde_json::to_vec(&crate::api::schema::ErrorResponse {
        id: id.into(),
        error: crate::api::schema::ErrorBody {
            code: "surface_rejected".into(),
            message: message.into(),
        },
    })
    .unwrap()
}

fn surface_set_active(message: &crate::protocol::ClientMessage) -> Option<bool> {
    let crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. } = message else {
        return None;
    };
    let request: crate::api::schema::Request = serde_json::from_str(request).ok()?;
    match request.method {
        crate::api::schema::Method::ClientShellSurfaceSet(params) => Some(params.active),
        _ => None,
    }
}

fn resize() -> crate::protocol::ClientMessage {
    crate::protocol::ClientMessage::ClientShellResize {
        cell_width_px: 8,
        cell_height_px: 16,
        surface_size: crate::protocol::ClientSurfaceSize { cols: 80, rows: 24 },
        pixel_mouse: false,
    }
}

fn surface(boot_id: &str, revision: u64, pane: &str) -> crate::protocol::PaneSurfaceFrame {
    crate::protocol::PaneSurfaceFrame {
        boot_id: boot_id.into(),
        projection_revision: revision,
        surface_revision: revision,
        frame: crate::protocol::FrameData {
            cells: Vec::new(),
            width: 80,
            height: 24,
            cursor: None,
            hyperlinks: Vec::new(),
            graphics: Vec::new(),
        },
        panes: vec![crate::protocol::PaneSurfacePane {
            pane_id: pane.into(),
            content_revision: revision,
            rect: crate::protocol::SurfaceRect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            inner_rect: crate::protocol::SurfaceRect {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            scrollbar_rect: None,
            scroll: None,
            focused: true,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            alternate_screen_active: false,
            pixel_width: 0,
            pixel_height: 0,
        }],
        splits: Vec::new(),
        popup: None,
        graphics: Default::default(),
    }
}

fn machine() -> PendingEndpointActivation {
    PendingEndpointActivation {
        source: lease(ClientEndpointId::Local, 1, "local-boot"),
        source_available: true,
        target: lease(endpoint(), 7, "remote-boot"),
        focus: None,
        host_focused: true,
        resize: resize(),
        phase: ActivationPhase::ActivatingTarget {
            request_id: "client-shell-surface:3:on".into(),
            acknowledged_revision: Some(1),
            focus_request_id: None,
            focus_request_target: None,
            focus_acknowledged: true,
            evidence: ActivationEvidence::default(),
        },
        deadline: Instant::now() + ACTIVATION_TIMEOUT,
        epoch: 3,
        next_focus_serial: 0,
        rollback_error: None,
        successor: None,
    }
}

#[test]
fn source_off_request_is_distinct_and_precedes_target_on_phase() {
    let activation = PendingEndpointActivation {
        source: lease(ClientEndpointId::Local, 1, "local-boot"),
        source_available: true,
        target: lease(endpoint(), 7, "remote-boot"),
        focus: None,
        host_focused: true,
        resize: resize(),
        phase: ActivationPhase::ReleasingSource {
            request_id: "client-shell-surface:9:off".into(),
        },
        deadline: Instant::now() + ACTIVATION_TIMEOUT,
        epoch: 9,
        next_focus_serial: 0,
        rollback_error: None,
        successor: None,
    };
    assert!(activation.accepts_response(
        &ClientEndpointId::Local,
        1,
        "local-boot",
        "client-shell-surface:9:off"
    ));
    assert!(!activation.accepts_response(
        &endpoint(),
        7,
        "remote-boot",
        "client-shell-surface:9:on"
    ));
}

#[test]
fn active_source_requires_metadata_from_its_current_connection_generation() {
    let (mut shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
    shell.set_endpoint_snapshot_for_generation(
        &ClientEndpointId::Local,
        99,
        Box::new(test_snapshot("stale-local-boot", 1)),
    );

    let result = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        26,
        Instant::now(),
    );

    assert!(
        matches!(result, Err(ActivationBeginError::Preflight(message)) if message.contains("this connection"))
    );
    assert!(local_sent.lock().unwrap().is_empty());
    assert!(remote_sent.lock().unwrap().is_empty());
    assert!(endpoints.active_surface_available());
}

#[test]
fn observed_begin_write_failure_returns_recoverable_partial_activation() {
    let (shell, mut endpoints, local_sent, remote_sent) =
        shell_and_registry_with_source_failure(true);
    let result = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        10,
        Instant::now(),
    );

    let ActivationBeginError::Partial {
        mut activation,
        error,
    } = (match result {
        Err(error) => error,
        Ok(_) => panic!("an observed source write must return partial lifecycle state"),
    })
    else {
        panic!("an observed source write must return partial lifecycle state");
    };
    assert!(error.contains("focus revoke"));
    assert_eq!(
        local_sent.lock().unwrap().first(),
        Some(&crate::protocol::ClientMessage::ClientShellFocus { focused: false })
    );
    assert!(remote_sent.lock().unwrap().is_empty());
    assert!(matches!(
        activation.rollback(&mut endpoints, error, false),
        ActivationRollback::Unavailable(_)
    ));
}

#[test]
fn source_release_is_sent_and_acknowledged_before_target_activation() {
    let (shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        None,
        resize(),
        11,
        Instant::now(),
    )
    .unwrap();
    let local = local_sent.lock().unwrap();
    assert_eq!(
        local.first(),
        Some(&crate::protocol::ClientMessage::ClientShellFocus { focused: false }),
        "source focus is revoked before source-off"
    );
    assert_eq!(
        local
            .as_slice()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false],
        "the source is released first"
    );
    drop(local);
    assert!(remote_sent.lock().unwrap().is_empty());
    assert!(
        !endpoints.active_surface_available(),
        "pane input is blocked while frozen"
    );

    assert_eq!(
        activation.receive_response(
            &ClientEndpointId::Local,
            1,
            "client-shell-surface:11:off",
            &surface_success("client-shell-surface:11:off", false, 1),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Pending
    );
    let remote = remote_sent.lock().unwrap();
    assert!(matches!(
        remote[0],
        crate::protocol::ClientMessage::ClientShellResize { .. }
    ));
    assert_eq!(
        remote.get(2),
        Some(&crate::protocol::ClientMessage::ClientShellFocus { focused: true })
    );
    assert_eq!(remote.get(1).and_then(surface_set_active), Some(true));
}

#[test]
fn activation_requires_an_exact_snapshot_surface_revision_pair() {
    let mut activation = machine();
    let target = endpoint();
    let snapshot = crate::protocol::ClientShellSnapshot {
        boot_id: "remote-boot".into(),
        revision: 2,
        config_diagnostic: None,
        product_announcement: None,
        update_available: None,
        update_install_command: String::new(),
        server_keybindings_toml: None,
        latest_release_notes_available: false,
        integration_updates_available: false,
        worktree_directory: String::new(),
        release_notes: None,
        focused_workspace_id: None,
        focused_tab_id: None,
        focused_pane_id: None,
        tab_bar_right: Vec::new(),
        tab_bar_right_separator: String::new(),
        agent_view_label: None,
        agent_order: Vec::new(),
        workspaces: Vec::new(),
        tabs: Vec::new(),
        panes: Vec::new(),
        agents: Vec::new(),
        commands: Vec::new(),
        pinned_sections: Vec::new(),
    };
    assert_eq!(
        activation.receive_snapshot(&target, 7, &snapshot),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 1, "pane")),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 2, "pane")),
        SurfaceActivationProgress::Ready
    );
}

#[test]
fn typed_target_ack_sets_a_floor_for_same_boot_activation_evidence() {
    let (shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        None,
        resize(),
        16,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:16:off",
        &surface_success("client-shell-surface:16:off", false, 1),
        &mut endpoints,
    );
    assert_eq!(
        activation.receive_response(
            &target,
            7,
            "client-shell-surface:16:on",
            &surface_success("client-shell-surface:16:on", true, 4),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_snapshot(&target, 7, &test_snapshot("remote-boot", 3)),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 3, "pane")),
        SurfaceActivationProgress::Pending,
        "a delayed same-boot surface below the acknowledgement floor is not evidence"
    );
    assert_eq!(
        activation.receive_snapshot(&target, 7, &test_snapshot("remote-boot", 4)),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 4, "pane")),
        SurfaceActivationProgress::Ready
    );
}

/// Back/Forward history relies on this: the shell records the pane an
/// endpoint shows when it is presented, so an endpoint reached for one pane
/// must never be presented with the pane it had focused before.
#[test]
fn a_pane_targeted_activation_presents_the_target_only_with_that_pane_focused() {
    let (mut shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    let target = endpoint();
    let focused_on = |revision: u64, pane_id: &str| {
        let mut snapshot = test_snapshot("remote-boot", revision);
        snapshot.focused_pane_id = Some(pane_id.into());
        snapshot
    };
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
            "wanted".into(),
        )),
        resize(),
        30,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:30:off",
        &surface_success("client-shell-surface:30:off", false, 1),
        &mut endpoints,
    );
    let _ = activation.receive_response(
        &target,
        7,
        "client-shell-surface:30:on",
        &surface_success("client-shell-surface:30:on", true, 2),
        &mut endpoints,
    );

    // The target answers with what it had focused: a coherent snapshot and
    // surface, but of another pane.
    let retained = focused_on(2, "retained");
    shell.cache_endpoint_snapshot_for_generation(&target, 7, Box::new(retained.clone()));
    assert_eq!(
        activation.receive_snapshot(&target, 7, &retained),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 2, "retained")),
        SurfaceActivationProgress::Pending
    );
    // The focus request is acknowledged before a snapshot shows its effect.
    assert_eq!(
        activation.receive_response(
            &target,
            7,
            "client-shell-focus:30:1",
            &pane_focus_success("client-shell-focus:30:1", "wanted"),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Pending
    );
    assert!(shell.endpoint_is_active(&ClientEndpointId::Local));

    let arrived = focused_on(3, "wanted");
    shell.cache_endpoint_snapshot_for_generation(&target, 7, Box::new(arrived.clone()));
    assert_eq!(
        activation.receive_snapshot(&target, 7, &arrived),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&target, 7, surface("remote-boot", 3, "wanted")),
        SurfaceActivationProgress::Ready
    );
    assert!(matches!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::AwaitingPresentationSync { .. })
    ));
    // What is presented is the snapshot that shows the wanted pane.
    assert!(shell.endpoint_is_active(&target));
    assert!(shell.endpoint_snapshot_matches(&target, 7, "remote-boot", 3));
}

#[test]
fn stale_generation_and_boot_are_not_activation_evidence() {
    let mut activation = machine();
    assert_eq!(
        activation.receive_surface(&endpoint(), 6, surface("remote-boot", 1, "pane")),
        SurfaceActivationProgress::Stale
    );
    assert_eq!(
        activation.receive_surface(&endpoint(), 7, surface("old-boot", 1, "pane")),
        SurfaceActivationProgress::Stale
    );
}

#[test]
fn stale_response_boot_is_not_consumed() {
    let mut activation = machine();
    let (_shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    assert_eq!(
        activation.receive_response_for_boot(
            &endpoint(),
            7,
            "old-boot",
            "client-shell-surface:3:on",
            &surface_success("client-shell-surface:3:on", true, 2),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Stale
    );
}

#[test]
fn same_target_retarget_is_latest_wins() {
    let (shell, mut endpoints, _local_sent, remote_sent) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        Some(crate::client::shell::ClientEndpointFocusTarget::Workspace(
            "old".into(),
        )),
        resize(),
        12,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:12:off",
        &surface_success("client-shell-surface:12:off", false, 1),
        &mut endpoints,
    );
    let old_focus = remote_sent
        .lock()
        .unwrap()
        .iter()
        .find_map(|message| match message {
            crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. }
                if surface_set_active(message).is_none() =>
            {
                Some(
                    serde_json::from_str::<crate::api::schema::Request>(request)
                        .unwrap()
                        .id,
                )
            }
            _ => None,
        })
        .unwrap();
    let sent_before_retarget = remote_sent.lock().unwrap().len();
    activation
        .retarget(
            Some(crate::client::shell::ClientEndpointFocusTarget::Workspace(
                "new".into(),
            )),
            &mut endpoints,
        )
        .unwrap();
    assert_eq!(remote_sent.lock().unwrap().len(), sent_before_retarget);
    assert!(activation.accepts_response(&target, 7, "remote-boot", &old_focus));
    assert_eq!(
        activation.receive_response(
            &target,
            7,
            &old_focus,
            &workspace_focus_success(&old_focus, "old"),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Pending
    );
    let latest_focus = remote_sent
        .lock()
        .unwrap()
        .last()
        .and_then(|message| match message {
            crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. } => Some(
                serde_json::from_str::<crate::api::schema::Request>(request)
                    .unwrap()
                    .id,
            ),
            _ => None,
        })
        .unwrap();
    assert_ne!(latest_focus, old_focus);
    assert!(activation.accepts_response(&target, 7, "remote-boot", &latest_focus));
}

#[test]
fn latest_host_focus_is_replayed_to_the_eventual_target() {
    let (shell, mut endpoints, _local_sent, remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        25,
        Instant::now(),
    )
    .unwrap();
    activation.update_host_focus(false, &mut endpoints).unwrap();
    assert!(remote_sent.lock().unwrap().is_empty());

    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:25:off",
        &surface_success("client-shell-surface:25:off", false, 1),
        &mut endpoints,
    );
    assert_eq!(
        remote_sent.lock().unwrap().get(2),
        Some(&crate::protocol::ClientMessage::ClientShellFocus { focused: false })
    );

    activation.update_host_focus(true, &mut endpoints).unwrap();
    assert_eq!(
        remote_sent.lock().unwrap().last(),
        Some(&crate::protocol::ClientMessage::ClientShellFocus { focused: true })
    );
}

#[test]
fn host_focus_change_restarts_an_issued_presentation_effects_fence() {
    let (_shell, mut endpoints, _local_sent, remote_sent) = shell_and_registry();
    let mut activation = machine();
    activation.phase = ActivationPhase::AwaitingPresentationEffects {
        lease: lease(endpoint(), 7, "remote-boot"),
        token: "old-token".into(),
        ready: false,
        completion: Box::new(ActivationCompletion::Activated),
    };

    activation.update_host_focus(false, &mut endpoints).unwrap();

    assert!(matches!(
        activation.phase,
        ActivationPhase::SynchronizingPresentation { .. }
    ));
    assert_eq!(
        activation.receive_presentation_effects_ready(&endpoint(), 7, "old-token"),
        SurfaceActivationProgress::Stale
    );
    assert_eq!(
        remote_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![true]
    );
}

#[test]
fn source_release_rejection_restores_the_source_coherently() {
    let (shell, mut endpoints, local_sent, _remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        13,
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        activation.receive_response(
            &ClientEndpointId::Local,
            1,
            "client-shell-surface:13:off",
            &failure("client-shell-surface:13:off", "source rejected release"),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Rejected {
            message: "source rejected release".into(),
            source_release_rejected: true,
        }
    );
    assert_eq!(
        activation.rollback(&mut endpoints, "source rejected release".into(), true),
        ActivationRollback::Pending
    );
    assert_eq!(endpoints.active_id(), &ClientEndpointId::Local);
    assert!(!endpoints.active_surface_available());
    assert_eq!(
        local_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
}

#[test]
fn source_release_timeout_starts_an_acknowledged_source_restore() {
    let (shell, mut endpoints, local_sent, _remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        14,
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        activation.rollback(&mut endpoints, "source release timed out".into(), false),
        ActivationRollback::Pending
    );
    let sent = local_sent.lock().unwrap();
    assert_eq!(
        sent.iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
    assert!(activation.accepts_response(
        &ClientEndpointId::Local,
        1,
        "local-boot",
        "client-shell-surface:14:rollback-source-on"
    ));
}

#[test]
fn resize_invalidates_already_recorded_surface_evidence() {
    let (_shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    let mut activation = machine();
    assert_eq!(
        activation.receive_snapshot(&endpoint(), 7, &test_snapshot("remote-boot", 1)),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(&endpoint(), 7, surface("remote-boot", 1, "pane")),
        SurfaceActivationProgress::Ready
    );
    let resize = crate::protocol::ClientMessage::ClientShellResize {
        cell_width_px: 9,
        cell_height_px: 17,
        surface_size: crate::protocol::ClientSurfaceSize {
            cols: 100,
            rows: 30,
        },
        pixel_mouse: true,
    };
    activation.update_resize(resize, &mut endpoints).unwrap();
    assert_eq!(activation.progress(), SurfaceActivationProgress::Pending);
}

#[test]
fn resize_during_activation_reaches_the_pending_target() {
    let (shell, mut endpoints, _local_sent, remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        15,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:15:off",
        &surface_success("client-shell-surface:15:off", false, 1),
        &mut endpoints,
    );
    let resized = crate::protocol::ClientMessage::ClientShellResize {
        cell_width_px: 9,
        cell_height_px: 17,
        surface_size: crate::protocol::ClientSurfaceSize {
            cols: 100,
            rows: 30,
        },
        pixel_mouse: true,
    };
    activation
        .update_resize(resized.clone(), &mut endpoints)
        .unwrap();
    assert_eq!(remote_sent.lock().unwrap().last(), Some(&resized));
    assert_eq!(
        activation.receive_surface(&endpoint(), 7, surface("remote-boot", 1, "pane")),
        SurfaceActivationProgress::Pending,
        "a surface for the prior geometry cannot commit"
    );
}

#[test]
fn rapid_a_to_b_to_a_restores_source_before_a_fresh_latest_epoch() {
    let (mut shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        20,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:20:off",
        &surface_success("client-shell-surface:20:off", false, 1),
        &mut endpoints,
    );

    // The latest A-qualified target replaces B while B may already have accepted target-on.
    assert_eq!(
        activation.supersede(
            ClientEndpointId::Local,
            Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "local-pane".into(),
            )),
            &mut endpoints,
        ),
        ActivationRollback::Pending
    );
    assert_eq!(
        remote_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![true, false],
        "B is released before A can be restored"
    );
    assert_eq!(
        activation.receive_surface(&endpoint(), 7, surface("remote-boot", 2, "pane")),
        SurfaceActivationProgress::Stale,
        "delayed B activation evidence cannot satisfy A restoration"
    );
    let _ = activation.receive_response(
        &endpoint(),
        7,
        "client-shell-surface:20:rollback-target-off",
        &surface_success("client-shell-surface:20:rollback-target-off", false, 1),
        &mut endpoints,
    );
    assert_eq!(
        local_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false, true],
        "source restoration is acknowledged rather than racing target ownership"
    );
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:20:rollback-source-on",
        &surface_success("client-shell-surface:20:rollback-source-on", true, 2),
        &mut endpoints,
    );
    let local_snapshot = test_snapshot("local-boot", 2);
    shell.set_snapshot(Box::new(local_snapshot.clone()));
    assert_eq!(
        activation.receive_snapshot(&ClientEndpointId::Local, 1, &local_snapshot),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(
            &ClientEndpointId::Local,
            1,
            surface("local-boot", 2, "pane")
        ),
        SurfaceActivationProgress::Ready
    );
    assert!(matches!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::AwaitingPresentationSync {
            endpoint: ClientEndpointId::Local,
            ..
        })
    ));
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:20:presentation-sync",
        &surface_success("client-shell-surface:20:presentation-sync", true, 3),
        &mut endpoints,
    );
    let sync_snapshot = test_snapshot("local-boot", 3);
    shell.set_endpoint_snapshot_for_generation(
        &ClientEndpointId::Local,
        1,
        Box::new(sync_snapshot.clone()),
    );
    let _ = activation.receive_snapshot(&ClientEndpointId::Local, 1, &sync_snapshot);
    assert_eq!(
        activation.receive_surface(
            &ClientEndpointId::Local,
            1,
            surface("local-boot", 3, "pane")
        ),
        SurfaceActivationProgress::Ready
    );
    assert_eq!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::AwaitingPresentationEffects)
    );
    assert_eq!(
        activation.receive_presentation_effects_ready(
            &ClientEndpointId::Local,
            1,
            "20:1:local-boot"
        ),
        SurfaceActivationProgress::Ready
    );
    assert!(matches!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::RestoredSource { .. })
    ));
    assert!(matches!(
        activation.take_successor(),
        Some(EndpointActivationIntent {
            endpoint_id: ClientEndpointId::Local,
            ..
        })
    ));
    assert_eq!(endpoints.active_id(), &ClientEndpointId::Local);

    // Runtime queues this successor with force=true, so even source==target receives a new
    // activation epoch only after restoration committed.
    let _fresh_epoch = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        ClientEndpointId::Local,
        Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
            "local-pane".into(),
        )),
        resize(),
        21,
        Instant::now(),
    )
    .unwrap();
    assert!(local_sent.lock().unwrap().iter().any(|message| {
        matches!(
            message,
            crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. }
                if serde_json::from_str::<crate::api::schema::Request>(request)
                    .is_ok_and(|request| request.id == "client-shell-surface:21:on")
        )
    }));
}

#[test]
fn local_activation_does_not_wait_for_a_disconnected_stalled_or_failed_remote() {
    for source_state in ["disconnected", "stalled", "write-failed"] {
        local_escape(source_state);
    }
}

fn local_escape(source_state: &str) {
    let (mut shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
    let disconnected = endpoint();
    endpoints.set_surface_active(&ClientEndpointId::Local, false);
    endpoints.set_surface_active(&disconnected, true);
    assert!(endpoints.set_active(&disconnected));
    assert!(shell.activate_endpoint_projection(&disconnected));
    match source_state {
        "disconnected" => endpoints.disconnect(&disconnected),
        "write-failed" => endpoints.insert(
            disconnected.clone(),
            FakeTransport {
                sent: remote_sent,
                fail_after_write: true,
            },
            7,
            negotiation(),
            true,
        ),
        _ => {}
    }

    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        ClientEndpointId::Local,
        None,
        resize(),
        22,
        Instant::now(),
    )
    .unwrap();
    let local_activations = local_sent
        .lock()
        .unwrap()
        .iter()
        .filter_map(surface_set_active)
        .collect::<Vec<_>>();
    assert_eq!(local_activations, vec![true], "Local must not wait for SSH");
    assert!(!endpoints.active_surface_available());
    assert_eq!(endpoints.active_id(), &disconnected);
    assert_eq!(
        activation.receive_response(
            &disconnected,
            7,
            "client-shell-surface:22:off",
            &surface_success("client-shell-surface:22:off", false, 1),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Stale
    );
    assert_eq!(
        activation.receive_surface(&disconnected, 7, surface("remote-boot", 2, "stale")),
        SurfaceActivationProgress::Stale
    );

    assert_eq!(
        activation.receive_response(
            &ClientEndpointId::Local,
            1,
            "client-shell-surface:22:on",
            &surface_success("client-shell-surface:22:on", true, 2),
            &mut endpoints,
        ),
        SurfaceActivationProgress::Pending
    );
    let snapshot = test_snapshot("local-boot", 2);
    shell.set_endpoint_snapshot_for_generation(
        &ClientEndpointId::Local,
        1,
        Box::new(snapshot.clone()),
    );
    assert_eq!(
        activation.receive_snapshot(&ClientEndpointId::Local, 1, &snapshot),
        SurfaceActivationProgress::Pending
    );
    assert_eq!(
        activation.receive_surface(
            &ClientEndpointId::Local,
            1,
            surface("local-boot", 2, "pane")
        ),
        SurfaceActivationProgress::Ready
    );
    assert!(matches!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::AwaitingPresentationSync {
            previous,
            endpoint: ClientEndpointId::Local,
        }) if previous == disconnected
    ));
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:22:presentation-sync",
        &surface_success("client-shell-surface:22:presentation-sync", true, 3),
        &mut endpoints,
    );
    let sync_snapshot = test_snapshot("local-boot", 3);
    shell.set_endpoint_snapshot_for_generation(
        &ClientEndpointId::Local,
        1,
        Box::new(sync_snapshot.clone()),
    );
    let _ = activation.receive_snapshot(&ClientEndpointId::Local, 1, &sync_snapshot);
    assert_eq!(
        activation.receive_surface(
            &ClientEndpointId::Local,
            1,
            surface("local-boot", 3, "pane")
        ),
        SurfaceActivationProgress::Ready
    );
    assert_eq!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::AwaitingPresentationEffects)
    );
    assert_eq!(
        activation.receive_presentation_effects_ready(
            &ClientEndpointId::Local,
            1,
            "22:1:local-boot"
        ),
        SurfaceActivationProgress::Ready
    );
    assert_eq!(
        activation.complete(&mut shell, &mut endpoints),
        Ok(ActivationCompletion::Activated)
    );
    assert_eq!(endpoints.active_id(), &ClientEndpointId::Local);
    assert_ne!(endpoints.active_id(), &disconnected);
    assert!(
        !endpoints.active_surface_available(),
        "runtime opens input only after the effects fence"
    );
}

#[test]
fn local_selection_abandons_every_unfinished_remote_handoff_phase() {
    use crate::client::{
        endpoint_commands::EndpointCommands, shell_runtime::begin_endpoint_activation, ClientState,
    };
    for phase in ["release", "target", "rollback", "restore"] {
        let (shell, mut endpoints, local_sent, _remote_sent) = shell_and_registry();
        let mut abandoned = PendingEndpointActivation::begin(
            &shell,
            &mut endpoints,
            endpoint(),
            None,
            resize(),
            30,
            Instant::now(),
        )
        .unwrap();
        if phase != "release" {
            abandoned.receive_response(
                &ClientEndpointId::Local,
                1,
                "client-shell-surface:30:off",
                &surface_success("client-shell-surface:30:off", false, 1),
                &mut endpoints,
            );
        }
        if matches!(phase, "rollback" | "restore") {
            assert_eq!(
                abandoned.rollback(&mut endpoints, "cancel".into(), false),
                ActivationRollback::Pending
            );
        }
        if phase == "restore" {
            abandoned.receive_response(
                &endpoint(),
                7,
                "client-shell-surface:30:rollback-target-off",
                &surface_success("client-shell-surface:30:rollback-target-off", false, 1),
                &mut endpoints,
            );
        }
        local_sent.lock().unwrap().clear();
        let mut state = ClientState::test_new();
        state.shell = Some(shell);
        let mut commands = EndpointCommands::default();
        commands.enqueue(
            ClientEndpointId::Local,
            1,
            "local-boot".into(),
            Box::new(crate::api::schema::Request {
                id: "abandoned-workspace-focus".into(),
                method: crate::api::schema::Method::WorkspaceFocus(
                    crate::api::schema::WorkspaceTarget {
                        workspace_id: "stale-workspace".into(),
                    },
                ),
            }),
        );
        let mut pending = Some(abandoned);
        let mut serial = 31;
        let mut scheduled = None;
        for _ in 0..2 {
            begin_endpoint_activation(
                &mut state,
                &mut endpoints,
                &mut commands,
                &mut pending,
                &mut serial,
                ClientEndpointId::Local,
                None,
                false,
                Instant::now(),
                &mut scheduled,
            )
            .unwrap();
        }
        assert_eq!(serial, 32, "repeated Local selection must coalesce");
        let local = pending.as_mut().unwrap();
        assert_eq!(local.target(), &ClientEndpointId::Local);
        assert!(!endpoints.active_surface_available());
        assert!(state.presentation_frozen);
        assert!(commands
            .send_next(&ClientEndpointId::Local, &mut endpoints)
            .is_empty());
        assert!(!local_sent.lock().unwrap().iter().any(|message| {
            matches!(message, crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. }
                if serde_json::from_str::<crate::api::schema::Request>(request)
                    .is_ok_and(|request| request.id == "abandoned-workspace-focus"))
        }), "a fresh activation must discard focus retained by an older target epoch");
        let activations = local_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>();
        assert_eq!(activations, vec![false, true], "phase: {phase}");
        assert!(!local.accepts_response(
            &ClientEndpointId::Local,
            1,
            "local-boot",
            "client-shell-surface:30:off"
        ));
        assert_eq!(
            local.receive_surface(&endpoint(), 7, surface("remote-boot", 2, "stale")),
            SurfaceActivationProgress::Stale
        );
    }
}

#[test]
fn local_selection_waits_for_fresh_metadata_without_abandoning_remote() {
    use crate::client::{
        endpoint_commands::EndpointCommands,
        shell_runtime::{begin_endpoint_activation, take_ready_local_activation},
        ClientLoopEvent, ClientState,
    };
    for replaced_generation in [false, true] {
        let (mut shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
        shell.set_endpoint_snapshot_for_generation(
            &ClientEndpointId::Local,
            1,
            Box::new(test_snapshot("local-boot", 1)),
        );
        let mut state = ClientState::test_new();
        state.shell = Some(shell);
        let mut commands = EndpointCommands::default();
        let mut pending = None;
        let mut serial = 40;
        let mut scheduled = None;
        begin_endpoint_activation(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut serial,
            endpoint(),
            None,
            false,
            Instant::now(),
            &mut scheduled,
        )
        .unwrap();
        if replaced_generation {
            endpoints.insert(
                ClientEndpointId::Local,
                FakeTransport {
                    sent: local_sent.clone(),
                    fail_after_write: false,
                },
                2,
                negotiation(),
                false,
            );
        } else {
            endpoints.disconnect(&ClientEndpointId::Local);
        }
        begin_endpoint_activation(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut serial,
            ClientEndpointId::Local,
            Some(crate::client::shell::ClientEndpointFocusTarget::Workspace(
                "selected-local".into(),
            )),
            false,
            Instant::now(),
            &mut scheduled,
        )
        .unwrap();
        assert_eq!(
            pending.as_ref().map(PendingEndpointActivation::target),
            Some(&endpoint())
        );
        assert!(remote_sent.lock().unwrap().is_empty());
        assert_eq!(serial, 41);
        assert!(state.deferred_local_activation.is_some());
        assert!(take_ready_local_activation(&mut state, &endpoints).is_none());
        if !replaced_generation {
            endpoints.insert(
                ClientEndpointId::Local,
                FakeTransport {
                    sent: local_sent,
                    fail_after_write: false,
                },
                2,
                negotiation(),
                false,
            );
        }
        assert!(take_ready_local_activation(&mut state, &endpoints).is_none());
        state
            .shell
            .as_mut()
            .unwrap()
            .cache_endpoint_snapshot_for_generation(
                &ClientEndpointId::Local,
                2,
                Box::new(test_snapshot("local-boot", 1)),
            );
        let event = take_ready_local_activation(&mut state, &endpoints).unwrap();
        let ClientLoopEvent::ActivateEndpoint {
            endpoint_id,
            target,
            force,
        } = event
        else {
            panic!("expected retained Local selection");
        };
        assert_eq!(endpoint_id, ClientEndpointId::Local);
        assert_eq!(
            target,
            Some(crate::client::shell::ClientEndpointFocusTarget::Workspace(
                "selected-local".into()
            ))
        );
        begin_endpoint_activation(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut serial,
            endpoint_id,
            target,
            force,
            Instant::now(),
            &mut scheduled,
        )
        .unwrap();
        assert_eq!(pending.as_ref().unwrap().target(), &ClientEndpointId::Local);
        assert_eq!(serial, 42);
        assert!(!endpoints.active_surface_available());
        assert!(state.deferred_local_activation.is_none());
    }
}

#[test]
fn newer_remote_selection_cancels_deferred_local_selection() {
    use crate::client::{
        endpoint_commands::EndpointCommands, shell_runtime::begin_endpoint_activation, ClientState,
    };
    let (shell, mut endpoints, _, _) = shell_and_registry();
    let mut state = ClientState::test_new();
    state.shell = Some(shell);
    let mut commands = EndpointCommands::default();
    let mut pending = None;
    let mut serial = 50;
    let mut scheduled = None;
    endpoints.disconnect(&ClientEndpointId::Local);
    for endpoint_id in [ClientEndpointId::Local, endpoint()] {
        begin_endpoint_activation(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut serial,
            endpoint_id.clone(),
            None,
            false,
            Instant::now(),
            &mut scheduled,
        )
        .unwrap();
        assert_eq!(
            state.deferred_local_activation.is_some(),
            endpoint_id.is_local()
        );
    }
    assert_eq!(pending.as_ref().unwrap().target(), &endpoint());
}

#[cfg(windows)]
#[test]
fn notification_successor_replay_rejects_a_restarted_destination() {
    use crate::client::shell::ClientEndpointFocusTarget;
    use crate::client::{
        endpoint_commands::EndpointCommands, shell_runtime::begin_endpoint_activation, ClientState,
    };
    for destination_boot in ["local-boot", "replacement-boot"] {
        let (shell, mut endpoints, local_sent, remote_sent) = shell_and_registry();
        let mut activation = PendingEndpointActivation::begin(
            &shell,
            &mut endpoints,
            endpoint(),
            None,
            resize(),
            70,
            Instant::now(),
        )
        .unwrap();
        activation.receive_response(
            &ClientEndpointId::Local,
            1,
            "client-shell-surface:70:off",
            &surface_success("client-shell-surface:70:off", false, 1),
            &mut endpoints,
        );
        activation.supersede(
            ClientEndpointId::Local,
            Some(ClientEndpointFocusTarget::Notification {
                pane_id: "pane_1".into(),
                boot_id: "local-boot".into(),
            }),
            &mut endpoints,
        );
        let intent = activation
            .successor
            .take()
            .expect("notification queued during handoff");
        let mut state = ClientState::test_new();
        state.shell = Some(shell);
        let mut snapshot = test_snapshot(destination_boot, 2);
        snapshot.panes.push(crate::protocol::ClientShellPane {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            label: None,
            cwd: None,
            foreground_cwd: None,
            focused: false,
            right_click_passthrough: false,
        });
        state
            .shell
            .as_mut()
            .unwrap()
            .set_snapshot(Box::new(snapshot));
        // Model restoration completing before the retained intent is replayed.
        endpoints.set_surface_active(&ClientEndpointId::Local, true);
        endpoints.unfreeze_input();
        local_sent.lock().unwrap().clear();
        remote_sent.lock().unwrap().clear();
        let mut pending = None;
        let mut commands = EndpointCommands::default();
        begin_endpoint_activation(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut 71,
            intent.endpoint_id,
            intent.target,
            true,
            Instant::now(),
            &mut None,
        )
        .unwrap();
        if destination_boot == "replacement-boot" {
            assert!(pending.is_none());
            assert!(
                local_sent.lock().unwrap().is_empty(),
                "stale successor must emit no navigation or surface write"
            );
            assert!(remote_sent.lock().unwrap().is_empty());
        } else {
            assert!(
                pending.is_some(),
                "same boot remains eligible after handoff"
            );
        }
    }
}

/// One client-loop turn for a jump a daemon forwarded: the shell decides the
/// navigation, and the loop dispatches it and begins the activation it
/// scheduled, if any.
fn receive_jump(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    serial: &mut u64,
    number: u64,
    endpoint_id: &ClientEndpointId,
    pane_id: &str,
) {
    use crate::client::shell_runtime;
    let now = Instant::now();
    let shell = state.shell.as_mut().unwrap();
    let boot_id = shell.endpoint_boot_id(endpoint_id).unwrap().to_owned();
    let input = shell.receive_client_jump(
        crate::client::shell::ControlOrigin {
            endpoint_id: ClientEndpointId::Local,
            generation: 1,
        },
        crate::protocol::endpoint::EndpointClientJump {
            jump_id: format!("jump-{number}"),
            endpoint_id: endpoint_id.storage_key(),
            boot_id,
            pane_id: pane_id.into(),
        },
        now,
    );
    let mut scheduled = None;
    shell_runtime::dispatch_client_shell_actions(
        input.actions,
        commands,
        endpoints,
        Some(shell),
        &mut state.detached_process_children,
        &mut scheduled,
    )
    .unwrap();
    begin_scheduled(state, endpoints, commands, pending, serial, scheduled);
}

/// Local and the remote, each with its `pane_1` focused and a `pane_2`.
fn shell_and_registry_with_panes() -> (crate::client::ClientState, EndpointRegistry) {
    let (state, endpoints, _) = shell_and_registry_with_panes_and_remote_sent();
    (state, endpoints)
}

/// The same, with what the client writes to the remote.
fn shell_and_registry_with_panes_and_remote_sent(
) -> (crate::client::ClientState, EndpointRegistry, SentMessages) {
    use crate::api::schema::AgentStatus;
    use crate::client::shell::tests::fleet::{agent, machine};
    let (mut shell, endpoints, _, remote_sent) = shell_and_registry();
    for (endpoint_id, boot_id) in [
        (ClientEndpointId::Local, "local-boot"),
        (endpoint(), "remote-boot"),
    ] {
        let mut snapshot = machine(
            boot_id,
            vec![
                agent("pane_1", AgentStatus::Idle, 1),
                agent("pane_2", AgentStatus::Idle, 2),
            ],
        );
        snapshot.revision = 2;
        shell.set_endpoint_snapshot(&endpoint_id, Box::new(snapshot));
    }
    let mut state = crate::client::ClientState::test_new();
    state.shell = Some(shell);
    (state, endpoints, remote_sent)
}

#[test]
fn a_jump_to_the_local_pane_still_shown_abandons_the_replaced_jumps_remote_handoff() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 60;

    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        1,
        &endpoint(),
        "pane_2",
    );
    assert_eq!(
        pending.as_ref().map(PendingEndpointActivation::target),
        Some(&endpoint())
    );

    // Local is still shown with `pane_1` focused, which the newer jump names.
    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        2,
        &ClientEndpointId::Local,
        "pane_1",
    );

    assert_eq!(
        pending.as_ref().map(PendingEndpointActivation::target),
        Some(&ClientEndpointId::Local),
        "the remote handoff must not complete after the newer jump"
    );
    // The jump is answered once Local's own activation has completed.
    assert_eq!(jump_answers(&mut state), []);
    let local = local_host();
    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-focus:61:1",
        &pane_focus_success("client-shell-focus:61:1", "pane_1"),
        &mut endpoints,
    );
    for (request_id, revision) in [
        ("client-shell-surface:61:on", 3),
        ("client-shell-surface:61:presentation-sync", 4),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &local,
            request_id,
            revision,
            "pane_1",
        );
        assert!(next.is_none());
        assert_eq!(jump_answers(&mut state), []);
    }
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        61,
    );
    assert!(next.is_none());
    assert!(pending.is_none());
    assert_eq!(
        jump_answers(&mut state),
        [(
            "jump-2".to_owned(),
            crate::api::schema::ClientJumpOutcome::Focused
        )]
    );
}

#[test]
fn a_jump_to_the_remote_pane_still_shown_supersedes_the_replaced_jumps_local_handoff() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let remote = endpoint();
    endpoints.set_surface_active(&ClientEndpointId::Local, false);
    endpoints.set_surface_active(&remote, true);
    assert!(endpoints.set_active(&remote));
    assert!(state
        .shell
        .as_mut()
        .unwrap()
        .activate_endpoint_projection(&remote));
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 70;

    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        1,
        &ClientEndpointId::Local,
        "pane_2",
    );
    assert_eq!(
        pending.as_ref().map(PendingEndpointActivation::target),
        Some(&ClientEndpointId::Local)
    );

    // The remote is still shown with `pane_1` focused, which the newer jump
    // names. The handoff to Local is reversed, and the jump's target is kept
    // as the intent to resume once the remote is restored.
    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        2,
        &remote,
        "pane_1",
    );

    assert_eq!(
        pending.as_ref().unwrap().successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote,
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_1".into()
            )),
        })
    );
}

/// The restored Local machine as it reports itself, with `pane_2` focused:
/// the focus moved there while the handoff was under way.
fn local_snapshot_with_pane_2_focused(revision: u64) -> crate::protocol::ClientShellSnapshot {
    use crate::api::schema::AgentStatus;
    use crate::client::shell::tests::fleet::{agent, machine};
    let mut snapshot = machine(
        "local-boot",
        vec![
            agent("pane_2", AgentStatus::Idle, 2),
            agent("pane_1", AgentStatus::Idle, 1),
        ],
    );
    snapshot.revision = revision;
    snapshot
}

/// Back/Forward history relies on this: a step to another machine leaves the
/// shell as an activation, which sends no request the shell tracks. When the
/// loop rolls the handoff back and nothing replaces it, the loop holds no
/// activation, and saying so is all that tells the shell the step will not
/// arrive.
#[test]
fn a_rolled_back_handoff_with_no_successor_ends_the_history_step_that_began_it() {
    use crate::client::{shell_runtime, ClientLoopEvent};
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let local = ClientEndpointId::Local;
    let remote = endpoint();
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 80;
    let mut scheduled = None;

    // Visited so far: Local `pane_1`, the remote's `pane_2`, Local `pane_1`.
    let shell = state.shell.as_mut().unwrap();
    shell
        .history
        .observe(&remote, "remote-boot", Some("pane_2"));
    shell.history.observe(&local, "local-boot", Some("pane_1"));

    // Back: the shell asks the loop to activate the remote.
    let size = shell.surface_size(state.reported_size.0, state.reported_size.1);
    let input = crate::client::shell::tests::history::step(shell, false);
    shell_runtime::dispatch_client_shell_actions(
        input.actions,
        &mut commands,
        &mut endpoints,
        Some(shell),
        &mut state.detached_process_children,
        &mut scheduled,
    )
    .unwrap();
    let Some(ClientLoopEvent::ActivateEndpoint {
        endpoint_id,
        target,
        force,
    }) = scheduled.take()
    else {
        panic!("the step asks for an activation");
    };
    assert_eq!(endpoint_id, remote);
    shell_runtime::begin_endpoint_activation(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        endpoint_id,
        target,
        force,
        Instant::now(),
        &mut scheduled,
    )
    .unwrap();

    // Local refuses to release its surface, so the loop restores it.
    let rejected = pending.as_mut().unwrap().receive_response(
        &local,
        1,
        "client-shell-surface:80:off",
        &failure("client-shell-surface:80:off", "source rejected release"),
        &mut endpoints,
    );
    assert_eq!(
        rejected,
        SurfaceActivationProgress::Rejected {
            message: "source rejected release".into(),
            source_release_rejected: true,
        }
    );
    shell_runtime::rollback_endpoint_activation(
        &mut state,
        &mut endpoints,
        &mut pending,
        "source rejected release".into(),
        true,
    );
    // Restoration is two coherent rounds, source-on and presentation sync,
    // each an acknowledgement, a snapshot and a surface, and then the
    // presentation effects fence.
    for (request_id, revision) in [
        ("client-shell-surface:80:rollback-source-on", 3),
        ("client-shell-surface:80:presentation-sync", 4),
    ] {
        let activation = pending.as_mut().unwrap();
        let _ = activation.receive_response(
            &local,
            1,
            request_id,
            &surface_success(request_id, true, revision),
            &mut endpoints,
        );
        let snapshot = local_snapshot_with_pane_2_focused(revision);
        state
            .shell
            .as_mut()
            .unwrap()
            .cache_endpoint_snapshot_for_generation(&local, 1, Box::new(snapshot.clone()));
        let _ = activation.receive_snapshot(&local, 1, &snapshot);
        let mut restored = surface("local-boot", revision, "pane_2");
        restored.frame.width = size.cols;
        restored.frame.height = size.rows;
        assert_eq!(
            activation.receive_surface(&local, 1, restored),
            SurfaceActivationProgress::Ready
        );
        let successor =
            complete_activation(&mut state, &mut endpoints, &mut commands, &mut pending);
        assert!(successor.is_none());
        assert!(pending.is_some(), "restoration is still under way");
    }
    // Local is presented again. The handoff completes once its presentation
    // effects are fenced, and no request is sent or answered after that.
    assert!(state.shell.as_ref().unwrap().endpoint_is_active(&local));
    assert_eq!(
        pending
            .as_mut()
            .unwrap()
            .receive_presentation_effects_ready(&local, 1, "80:1:local-boot"),
        SurfaceActivationProgress::Ready
    );
    let successor = complete_activation(&mut state, &mut endpoints, &mut commands, &mut pending);
    assert!(successor.is_none());
    assert!(pending.is_none(), "the handoff completed as a failure");

    // The cursor is back on Local `pane_1`, and `pane_2`, which went
    // unrecorded while the step was awaited, follows it. Nothing lies
    // ahead, one step back is `pane_1`, and the next is the remote again.
    let online = [local.clone(), remote.clone()].into_iter().collect();
    let shell = state.shell.as_mut().unwrap();
    assert!(shell.history.forward(&online).is_none());
    let back = shell.history.back(&online).unwrap();
    assert_eq!(
        (&back.visit.endpoint_id, back.visit.pane_id.as_str()),
        (&local, "pane_1")
    );
    let back = shell.history.back(&online).unwrap();
    assert_eq!(
        (&back.visit.endpoint_id, back.visit.pane_id.as_str()),
        (&remote, "pane_2")
    );
}

/// One client-loop turn for a Back or Forward key: the shell decides the
/// navigation, and the loop dispatches it and begins the activation it
/// scheduled, if any.
fn press_history_key(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    serial: &mut u64,
    forward: bool,
) {
    use crate::client::shell_runtime;
    let shell = state.shell.as_mut().unwrap();
    let input = crate::client::shell::tests::history::step(shell, forward);
    let mut scheduled = None;
    shell_runtime::dispatch_client_shell_actions(
        input.actions,
        commands,
        endpoints,
        Some(shell),
        &mut state.detached_process_children,
        &mut scheduled,
    )
    .unwrap();
    begin_scheduled(state, endpoints, commands, pending, serial, scheduled);
}

/// Back/Forward history relies on this as well: the loop freezes input for
/// a handoff, so it cancels a focus request sent to the machine still shown.
/// A second step to a pane on that machine therefore reaches the loop as an
/// activation, which reverses the first step's handoff and keeps the second
/// step as the intent to resume.
#[test]
fn a_history_step_to_the_remote_still_shown_supersedes_the_first_steps_local_handoff() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let local = ClientEndpointId::Local;
    let remote = endpoint();
    // Visited so far: Local `pane_1`, the remote's `pane_2`, Local `pane_1`,
    // and the remote's `pane_1`, which is shown.
    let shell = state.shell.as_mut().unwrap();
    shell
        .history
        .observe(&remote, "remote-boot", Some("pane_2"));
    shell.history.observe(&local, "local-boot", Some("pane_1"));
    endpoints.set_surface_active(&local, false);
    endpoints.set_surface_active(&remote, true);
    assert!(endpoints.set_active(&remote));
    assert!(shell.activate_endpoint_projection(&remote));
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 90;

    // Back: Local `pane_1`, a handoff.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    assert_eq!(
        pending.as_ref().map(PendingEndpointActivation::target),
        Some(&local)
    );

    // Back again before it ends: the remote's `pane_2`.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    assert_eq!(
        pending.as_ref().unwrap().successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote.clone(),
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_2".into()
            )),
        })
    );

    // The second step is still under way, two visits back from where the
    // first began: the next visit forward is Local `pane_1`.
    let online = [local.clone(), remote].into_iter().collect();
    let forward = state
        .shell
        .as_mut()
        .unwrap()
        .history
        .forward(&online)
        .unwrap();
    assert_eq!(
        (&forward.visit.endpoint_id, forward.visit.pane_id.as_str()),
        (&local, "pane_1")
    );
}

/// A machine as the fixtures connect it.
struct Host {
    id: ClientEndpointId,
    generation: u64,
    boot_id: &'static str,
}

fn local_host() -> Host {
    Host {
        id: ClientEndpointId::Local,
        generation: 1,
        boot_id: "local-boot",
    }
}

fn remote_host() -> Host {
    Host {
        id: endpoint(),
        generation: 7,
        boot_id: "remote-boot",
    }
}

/// A machine's snapshot at `revision`, with `pane_1` and `pane_2` and
/// `focused` focused.
fn snapshot_focused_on(
    boot_id: &str,
    focused: &str,
    revision: u64,
) -> crate::protocol::ClientShellSnapshot {
    use crate::api::schema::AgentStatus;
    use crate::client::shell::tests::fleet::{agent, machine};
    let other = if focused == "pane_1" {
        "pane_2"
    } else {
        "pane_1"
    };
    let mut snapshot = machine(
        boot_id,
        vec![
            agent(focused, AgentStatus::Idle, 1),
            agent(other, AgentStatus::Idle, 2),
        ],
    );
    snapshot.revision = revision;
    snapshot
}

/// The loop turn once an activation's evidence is ready, up to the top of the
/// next turn, where the loop tells the shell what it holds. Returns the
/// activation the loop scheduled for that turn, if any.
fn complete_activation(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
) -> Option<crate::client::ClientLoopEvent> {
    use crate::client::shell_runtime;
    let scheduled =
        shell_runtime::complete_endpoint_activation(state, endpoints, pending, commands).unwrap();
    shell_runtime::report_held_activations(state, pending.as_ref(), scheduled.as_ref());
    scheduled
}

/// One coherent round of a handoff: `host` acknowledges `request_id` and
/// sends a snapshot and a surface that show `focused`, and the loop takes
/// the activation as far as that allows. Returns the activation the loop
/// schedules next, if any.
fn coherent_round(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    host: &Host,
    request_id: &str,
    revision: u64,
    focused: &str,
) -> Option<crate::client::ClientLoopEvent> {
    let size = {
        let shell = state.shell.as_ref().unwrap();
        shell.surface_size(state.reported_size.0, state.reported_size.1)
    };
    let activation = pending.as_mut().unwrap();
    let _ = activation.receive_response(
        &host.id,
        host.generation,
        request_id,
        &surface_success(request_id, true, revision),
        endpoints,
    );
    let snapshot = snapshot_focused_on(host.boot_id, focused, revision);
    state
        .shell
        .as_mut()
        .unwrap()
        .cache_endpoint_snapshot_for_generation(
            &host.id,
            host.generation,
            Box::new(snapshot.clone()),
        );
    let _ = activation.receive_snapshot(&host.id, host.generation, &snapshot);
    let mut shown = surface(host.boot_id, revision, focused);
    shown.frame.width = size.cols;
    shown.frame.height = size.rows;
    assert_eq!(
        activation.receive_surface(&host.id, host.generation, shown),
        SurfaceActivationProgress::Ready
    );
    complete_activation(state, endpoints, commands, pending)
}

/// The last round of a handoff: `host` has replayed the presentation
/// effects of the activation numbered `serial`.
fn effects_fenced(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    host: &Host,
    serial: u64,
) -> Option<crate::client::ClientLoopEvent> {
    let token = format!("{serial}:{}:{}", host.generation, host.boot_id);
    assert_eq!(
        pending
            .as_mut()
            .unwrap()
            .receive_presentation_effects_ready(&host.id, host.generation, &token),
        SurfaceActivationProgress::Ready
    );
    complete_activation(state, endpoints, commands, pending)
}

/// The loop turn that follows one in which `scheduled` was left, up to the
/// top of the turn after it: the loop tells the shell what it holds, takes up
/// the activation it scheduled, if any, and tells the shell again.
fn begin_scheduled(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    serial: &mut u64,
    mut scheduled: Option<crate::client::ClientLoopEvent>,
) {
    use crate::client::shell_runtime;
    shell_runtime::report_held_activations(state, pending.as_ref(), scheduled.as_ref());
    if let Some(crate::client::ClientLoopEvent::ActivateEndpoint {
        endpoint_id,
        target,
        force,
    }) = scheduled.take()
    {
        shell_runtime::begin_endpoint_activation(
            state,
            endpoints,
            commands,
            pending,
            serial,
            endpoint_id,
            target,
            force,
            Instant::now(),
            &mut scheduled,
        )
        .unwrap();
    }
    shell_runtime::report_held_activations(state, pending.as_ref(), scheduled.as_ref());
}

/// The focus request the shell last sent to a machine through the loop's
/// command lane, as `(request id, pane id)`.
fn sent_focus_request(sent: &SentMessages) -> (String, String) {
    let request = sent
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|message| match message {
            crate::protocol::ClientMessage::ClientShellEndpointRequest { request, .. } => {
                serde_json::from_str::<crate::api::schema::Request>(request).ok()
            }
            _ => None,
        })
        .filter(|request| request.id.starts_with("client-shell:"))
        .expect("the shell's request was sent");
    let crate::api::schema::Method::PaneFocus(target) = request.method else {
        panic!("expected a pane focus request: {request:?}");
    };
    (request.id, target.pane_id)
}

/// The next visit forward in the shell's history, as `(machine, pane id)`.
/// Asking moves the cursor there, so a test asks last.
fn next_visit_forward(
    state: &mut crate::client::ClientState,
) -> Option<(ClientEndpointId, String)> {
    let online = [ClientEndpointId::Local, endpoint()].into_iter().collect();
    state
        .shell
        .as_mut()
        .unwrap()
        .history
        .forward(&online)
        .map(|step| (step.visit.endpoint_id, step.visit.pane_id))
}

/// Presents the remote with its `pane_1` focused, in place of Local.
fn present_remote(state: &mut crate::client::ClientState, endpoints: &mut EndpointRegistry) {
    let remote = endpoint();
    endpoints.set_surface_active(&ClientEndpointId::Local, false);
    endpoints.set_surface_active(&remote, true);
    assert!(endpoints.set_active(&remote));
    assert!(state
        .shell
        .as_mut()
        .unwrap()
        .activate_endpoint_projection(&remote));
}

/// Back/Forward history relies on this too: while a handoff is reversed for
/// a newer step, the machine it left is presented again before that step's
/// own activation begins. What the machine shows then says nothing about the
/// step, even a pane equal to where the step leads.
#[test]
fn a_machine_restored_ahead_of_a_successor_step_does_not_stand_for_that_step() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let (local, remote) = (local_host(), remote_host());
    // Visited so far: Local `pane_1`, the remote's `pane_1`, Local `pane_1`,
    // and the remote's `pane_1` again, which is shown.
    let shell = state.shell.as_mut().unwrap();
    shell
        .history
        .observe(&remote.id, remote.boot_id, Some("pane_1"));
    shell
        .history
        .observe(&local.id, local.boot_id, Some("pane_1"));
    present_remote(&mut state, &mut endpoints);
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 90;

    // Back to Local `pane_1`, a handoff, and Back again before it ends, to
    // the remote's first `pane_1`.
    for _ in 0..2 {
        press_history_key(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &mut serial,
            false,
        );
    }
    assert_eq!(
        pending.as_ref().unwrap().successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote.id.clone(),
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_1".into()
            )),
        })
    );

    // Local gives the surface back and the remote is restored: first as it
    // was, with `pane_1` focused, then with `pane_2`, which something else
    // focused meanwhile.
    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-surface:90:rollback-target-off",
        &surface_success("client-shell-surface:90:rollback-target-off", false, 3),
        &mut endpoints,
    );
    for (request_id, revision, focused) in [
        ("client-shell-surface:90:rollback-source-on", 3, "pane_1"),
        ("client-shell-surface:90:presentation-sync", 4, "pane_2"),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &remote,
            request_id,
            revision,
            focused,
        );
        assert!(next.is_none());
    }
    // Only now does the loop schedule the second step's own activation.
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        90,
    );
    assert!(matches!(
        next,
        Some(crate::client::ClientLoopEvent::ActivateEndpoint { force: true, .. })
    ));
    begin_scheduled(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        next,
    );
    let _ = pending.as_mut().unwrap().receive_response(
        &remote.id,
        remote.generation,
        "client-shell-focus:91:1",
        &pane_focus_success("client-shell-focus:91:1", "pane_1"),
        &mut endpoints,
    );
    for (request_id, revision) in [
        ("client-shell-surface:91:on", 5),
        ("client-shell-surface:91:presentation-sync", 6),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &remote,
            request_id,
            revision,
            "pane_1",
        );
        assert!(next.is_none());
    }
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        91,
    );
    assert!(next.is_none());
    assert!(pending.is_none());

    // The step arrived two visits back from where the chain began, and
    // `pane_2` was no visit: Local `pane_1` is still the next one forward.
    assert_eq!(
        next_visit_forward(&mut state),
        Some((local.id, "pane_1".to_owned()))
    );
}

/// A handoff shows its target two request rounds before it completes, and
/// the loop takes no request until then. A step pressed in between, to a
/// pane on the machine the handoff left, reverses the handoff: that machine
/// is presented again as it was, which is neither the step's arrival nor a
/// selection that replaced it.
#[test]
fn a_history_step_pressed_once_a_handoff_shows_its_target_keeps_the_forward_list() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let (local, remote) = (local_host(), remote_host());
    // Visited so far: Local `pane_1`, the remote's `pane_2`, Local `pane_1`,
    // and the remote's `pane_1`, which is shown.
    let shell = state.shell.as_mut().unwrap();
    shell
        .history
        .observe(&remote.id, remote.boot_id, Some("pane_2"));
    shell
        .history
        .observe(&local.id, local.boot_id, Some("pane_1"));
    present_remote(&mut state, &mut endpoints);
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 90;

    // Back: Local `pane_1`, a handoff, which shows Local with the pane
    // focused and goes on to synchronize its presentation.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-focus:90:1",
        &pane_focus_success("client-shell-focus:90:1", "pane_1"),
        &mut endpoints,
    );
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        "client-shell-surface:90:on",
        3,
        "pane_1",
    );
    assert!(next.is_none());
    assert!(pending.is_some());
    assert!(state.shell.as_ref().unwrap().endpoint_is_active(&local.id));

    // Back again: the remote's `pane_2`. The loop reverses the handoff.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    assert_eq!(
        pending.as_ref().unwrap().successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote.id.clone(),
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_2".into()
            )),
        })
    );
    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-surface:90:rollback-target-off",
        &surface_success("client-shell-surface:90:rollback-target-off", false, 4),
        &mut endpoints,
    );
    for (request_id, revision) in [
        ("client-shell-surface:90:rollback-source-on", 3),
        ("client-shell-surface:90:presentation-sync", 4),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &remote,
            request_id,
            revision,
            "pane_1",
        );
        assert!(next.is_none());
        assert!(state.shell.as_ref().unwrap().endpoint_is_active(&remote.id));
    }
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        90,
    );

    // The second step's own activation shows the remote's `pane_2`.
    begin_scheduled(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        next,
    );
    let _ = pending.as_mut().unwrap().receive_response(
        &remote.id,
        remote.generation,
        "client-shell-focus:91:1",
        &pane_focus_success("client-shell-focus:91:1", "pane_2"),
        &mut endpoints,
    );
    for (request_id, revision) in [
        ("client-shell-surface:91:on", 5),
        ("client-shell-surface:91:presentation-sync", 6),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &remote,
            request_id,
            revision,
            "pane_2",
        );
        assert!(next.is_none());
    }
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        91,
    );
    assert!(next.is_none());
    assert!(pending.is_none());

    // The cursor is two visits back from where the first step began, with
    // nothing recorded on the way: Local `pane_1` is the next visit forward.
    assert_eq!(
        next_visit_forward(&mut state),
        Some((local.id, "pane_1".to_owned()))
    );
}

/// The other step that can be pressed in that window leads to a pane on the
/// machine the handoff has just shown. The loop takes no request there yet
/// and nothing needs reversing, so the step waits for the handoff to
/// complete and is then sent as its own focus request.
#[test]
fn a_history_step_to_the_machine_a_handoff_just_showed_follows_the_handoff() {
    let (mut state, mut endpoints, remote_sent) = shell_and_registry_with_panes_and_remote_sent();
    let (local, remote) = (local_host(), remote_host());
    // Visited so far: Local `pane_1`, the remote's `pane_1` and `pane_2`,
    // and Local `pane_1`, which is shown.
    let shell = state.shell.as_mut().unwrap();
    for (host, pane_id) in [(&remote, "pane_1"), (&remote, "pane_2"), (&local, "pane_1")] {
        shell.history.observe(&host.id, host.boot_id, Some(pane_id));
    }
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 100;

    // Back: the remote's `pane_2`, a handoff, which shows the remote with
    // the pane focused.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    for (host, request_id, response) in [
        (
            &local,
            "client-shell-surface:100:off",
            surface_success("client-shell-surface:100:off", false, 3),
        ),
        (
            &remote,
            "client-shell-focus:100:1",
            pane_focus_success("client-shell-focus:100:1", "pane_2"),
        ),
    ] {
        let _ = pending.as_mut().unwrap().receive_response(
            &host.id,
            host.generation,
            request_id,
            &response,
            &mut endpoints,
        );
    }
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        "client-shell-surface:100:on",
        3,
        "pane_2",
    );
    assert!(next.is_none());
    assert!(state.shell.as_ref().unwrap().endpoint_is_active(&remote.id));

    // Back again: the remote's `pane_1`. The handoff goes on as it was, and
    // the step is kept as the intent to resume.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    let activation = pending.as_ref().unwrap();
    assert!(matches!(
        activation.phase,
        ActivationPhase::SynchronizingPresentation { .. }
    ));
    assert_eq!(
        activation.successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote.id.clone(),
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_1".into()
            )),
        })
    );
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        "client-shell-surface:100:presentation-sync",
        4,
        "pane_2",
    );
    assert!(next.is_none());
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        100,
    );
    assert!(matches!(
        next,
        Some(crate::client::ClientLoopEvent::ActivateEndpoint { force: false, .. })
    ));

    // With no handoff left, the loop routes the step back as its request.
    begin_scheduled(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        next,
    );
    assert!(pending.is_none());
    let (request_id, pane_id) = sent_focus_request(&remote_sent);
    assert_eq!(pane_id, "pane_1");
    let shell = state.shell.as_mut().unwrap();
    shell.handle_endpoint_result(
        remote.boot_id,
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    shell.set_endpoint_snapshot_for_generation(
        &remote.id,
        remote.generation,
        Box::new(snapshot_focused_on(remote.boot_id, "pane_1", 5)),
    );

    // The step arrived two visits back, and the remote's `pane_2` is the
    // next visit forward.
    assert_eq!(
        next_visit_forward(&mut state),
        Some((remote.id, "pane_2".to_owned()))
    );
}

/// When the machine a handoff has just shown is Local, the loop does not
/// wait: a selection of Local replaces any handoff with a fresh activation
/// of Local. The step is carried out by that one, and the handoff it
/// replaced never reports.
#[test]
fn a_history_step_to_local_just_shown_by_a_handoff_takes_the_handoffs_place() {
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let local = local_host();
    // Visited so far: Local `pane_1`, `pane_2` and `pane_1`, and the
    // remote's `pane_1`, which is shown.
    let shell = state.shell.as_mut().unwrap();
    for pane_id in ["pane_2", "pane_1"] {
        shell
            .history
            .observe(&local.id, local.boot_id, Some(pane_id));
    }
    present_remote(&mut state, &mut endpoints);
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 130;

    // Back: Local `pane_1`, a handoff, which shows Local with the pane
    // focused. Back again: Local `pane_2`.
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-focus:130:1",
        &pane_focus_success("client-shell-focus:130:1", "pane_1"),
        &mut endpoints,
    );
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        "client-shell-surface:130:on",
        3,
        "pane_1",
    );
    assert!(next.is_none());
    press_history_key(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        false,
    );
    assert_eq!(
        pending.as_ref().map(PendingEndpointActivation::target),
        Some(&local.id)
    );
    assert_eq!(pending.as_ref().unwrap().epoch, 131);

    let _ = pending.as_mut().unwrap().receive_response(
        &local.id,
        local.generation,
        "client-shell-focus:131:1",
        &pane_focus_success("client-shell-focus:131:1", "pane_2"),
        &mut endpoints,
    );
    for (request_id, revision) in [
        ("client-shell-surface:131:on", 4),
        ("client-shell-surface:131:presentation-sync", 5),
    ] {
        let next = coherent_round(
            &mut state,
            &mut endpoints,
            &mut commands,
            &mut pending,
            &local,
            request_id,
            revision,
            "pane_2",
        );
        assert!(next.is_none());
    }
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        131,
    );
    assert!(next.is_none());
    assert!(pending.is_none());

    // The step arrived two visits back: Local `pane_1` is the next forward.
    assert_eq!(
        next_visit_forward(&mut state),
        Some((local.id, "pane_1".to_owned()))
    );
}

/// The answers the shell has ready for the daemon, as `(jump id, outcome)`.
fn jump_answers(
    state: &mut crate::client::ClientState,
) -> Vec<(String, crate::api::schema::ClientJumpOutcome)> {
    state
        .shell
        .as_mut()
        .unwrap()
        .settle_client_jumps(Instant::now())
        .into_iter()
        .map(|answer| (answer.jump_id, answer.outcome))
        .collect()
}

/// Local is shown and a jump to the remote's `pane_2` begins a handoff,
/// which has shown the remote with the pane focused and still synchronizes
/// its presentation. The handoff is numbered 110.
fn jump_handoff_showing_its_target(
    state: &mut crate::client::ClientState,
    endpoints: &mut EndpointRegistry,
    commands: &mut crate::client::endpoint_commands::EndpointCommands,
    pending: &mut Option<PendingEndpointActivation>,
    serial: &mut u64,
) {
    let (local, remote) = (local_host(), remote_host());
    *serial = 110;
    receive_jump(
        state, endpoints, commands, pending, serial, 1, &remote.id, "pane_2",
    );
    for (host, request_id, response) in [
        (
            &local,
            "client-shell-surface:110:off",
            surface_success("client-shell-surface:110:off", false, 3),
        ),
        (
            &remote,
            "client-shell-focus:110:1",
            pane_focus_success("client-shell-focus:110:1", "pane_2"),
        ),
    ] {
        let _ = pending.as_mut().unwrap().receive_response(
            &host.id,
            host.generation,
            request_id,
            &response,
            endpoints,
        );
    }
    let next = coherent_round(
        state,
        endpoints,
        commands,
        pending,
        &remote,
        "client-shell-surface:110:on",
        3,
        "pane_2",
    );
    assert!(next.is_none());
    assert!(pending.is_some());
    assert!(state.shell.as_ref().unwrap().endpoint_is_active(&remote.id));
}

/// A tool may jump again as soon as its jump is answered. The loop takes no
/// request for the machine a handoff shows until the handoff completes, so
/// the answer waits for that: the next jump is then a focus request the
/// loop sends.
#[test]
fn a_jump_is_answered_focused_once_its_handoff_completes_and_the_next_jump_is_taken() {
    let (mut state, mut endpoints, remote_sent) = shell_and_registry_with_panes_and_remote_sent();
    let remote = remote_host();
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 0;
    jump_handoff_showing_its_target(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
    );
    assert_eq!(jump_answers(&mut state), []);

    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        "client-shell-surface:110:presentation-sync",
        4,
        "pane_2",
    );
    assert!(next.is_none());
    assert_eq!(jump_answers(&mut state), []);
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        110,
    );
    assert!(next.is_none());
    assert!(pending.is_none());
    assert_eq!(
        jump_answers(&mut state),
        [(
            "jump-1".to_owned(),
            crate::api::schema::ClientJumpOutcome::Focused
        )]
    );

    // The tool jumps on to the remote's `pane_1`.
    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        2,
        &remote.id,
        "pane_1",
    );
    assert_eq!(sent_focus_request(&remote_sent).1, "pane_1");
    assert_eq!(jump_answers(&mut state), []);
}

/// A jump that arrives in that window, to another pane of the machine the
/// handoff has just shown, replaces the open one. It waits for the handoff
/// to complete, as a history step does, and is then sent as its own focus
/// request.
#[test]
fn a_jump_to_the_machine_a_handoff_just_showed_follows_the_handoff() {
    let (mut state, mut endpoints, remote_sent) = shell_and_registry_with_panes_and_remote_sent();
    let remote = remote_host();
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 0;
    jump_handoff_showing_its_target(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
    );

    receive_jump(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        2,
        &remote.id,
        "pane_1",
    );
    assert_eq!(jump_answers(&mut state), []);
    assert!(matches!(
        pending.as_ref().unwrap().phase,
        ActivationPhase::SynchronizingPresentation { .. }
    ));

    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        "client-shell-surface:110:presentation-sync",
        4,
        "pane_2",
    );
    assert!(next.is_none());
    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &remote,
        110,
    );
    assert_eq!(jump_answers(&mut state), []);
    begin_scheduled(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        next,
    );
    let (request_id, pane_id) = sent_focus_request(&remote_sent);
    assert_eq!(pane_id, "pane_1");
    assert_eq!(jump_answers(&mut state), []);

    // The remote carries the request out and shows the pane.
    let shell = state.shell.as_mut().unwrap();
    shell.handle_endpoint_result(
        remote.boot_id,
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    shell.set_endpoint_snapshot_for_generation(
        &remote.id,
        remote.generation,
        Box::new(snapshot_focused_on(remote.boot_id, "pane_1", 5)),
    );
    assert_eq!(
        jump_answers(&mut state),
        [(
            "jump-2".to_owned(),
            crate::api::schema::ClientJumpOutcome::Focused
        )]
    );
}

/// Once the source is being restored, a newer selection only replaces the
/// intent to resume, through every round of the restoration, and the latest
/// one is what the loop starts when the restoration completes.
#[test]
fn the_latest_selection_made_while_the_source_is_restored_is_the_one_resumed() {
    use crate::client::shell::ClientEndpointFocusTarget;
    use crate::client::{shell_runtime, ClientLoopEvent};
    let (mut state, mut endpoints) = shell_and_registry_with_panes();
    let (local, remote) = (local_host(), remote_host());
    let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
    let mut pending = None;
    let mut serial = 120;
    let select = |state: &mut crate::client::ClientState,
                  endpoints: &mut EndpointRegistry,
                  commands: &mut crate::client::endpoint_commands::EndpointCommands,
                  pending: &mut Option<PendingEndpointActivation>,
                  serial: &mut u64,
                  pane_id: Option<&str>| {
        begin_scheduled(
            state,
            endpoints,
            commands,
            pending,
            serial,
            Some(ClientLoopEvent::ActivateEndpoint {
                endpoint_id: endpoint(),
                target: pane_id.map(|pane_id| ClientEndpointFocusTarget::Pane(pane_id.into())),
                force: false,
            }),
        );
    };

    // A selection of the remote begins a handoff. Local refuses to release
    // its surface, so the loop restores it.
    select(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        None,
    );
    shell_runtime::rollback_endpoint_activation(
        &mut state,
        &mut endpoints,
        &mut pending,
        "source rejected release".into(),
        true,
    );

    // One selection while Local synchronizes its presentation, and a newer
    // one while it replays its presentation effects.
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        "client-shell-surface:120:rollback-source-on",
        3,
        "pane_1",
    );
    assert!(next.is_none());
    select(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        Some("pane_1"),
    );
    let next = coherent_round(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        "client-shell-surface:120:presentation-sync",
        4,
        "pane_1",
    );
    assert!(next.is_none());
    select(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut serial,
        Some("pane_2"),
    );
    assert!(pending.is_some(), "the restoration goes on");

    let next = effects_fenced(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &local,
        120,
    );
    let Some(ClientLoopEvent::ActivateEndpoint {
        endpoint_id,
        target,
        force,
    }) = next
    else {
        panic!("the loop resumes the latest selection");
    };
    assert_eq!(
        (endpoint_id, target, force),
        (
            remote.id,
            Some(ClientEndpointFocusTarget::Pane("pane_2".into())),
            true
        )
    );
}

#[test]
fn rollback_keeps_the_latest_intent_even_when_it_returns_to_the_target() {
    let (shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        None,
        resize(),
        23,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:23:off",
        &surface_success("client-shell-surface:23:off", false, 1),
        &mut endpoints,
    );
    assert_eq!(
        activation.supersede(
            ClientEndpointId::Local,
            Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "local-pane".into()
            )),
            &mut endpoints,
        ),
        ActivationRollback::Pending
    );
    assert!(!activation.can_retarget(&target));
    assert_eq!(
        activation.supersede(
            target.clone(),
            Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "remote-pane".into()
            )),
            &mut endpoints,
        ),
        ActivationRollback::Pending
    );
    assert_eq!(
        activation.successor,
        Some(EndpointActivationIntent {
            endpoint_id: target,
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "remote-pane".into()
            )),
        })
    );
}

#[cfg(windows)]
#[test]
fn notification_click_on_remote_source_supersedes_a_handoff() {
    use crate::client::shell::{ClientEndpointFocusTarget, ClientSystemNotificationTarget};
    use crate::client::{
        endpoint_commands::EndpointCommands,
        events::ClientLoopEvent,
        shell_runtime::{begin_endpoint_activation, dispatch_client_shell_actions},
        ClientState,
    };
    let (mut shell, mut endpoints, _, _) = shell_and_registry();
    let remote = endpoint();
    let mut snapshot = test_snapshot("remote-boot", 2);
    snapshot.panes.push(crate::protocol::ClientShellPane {
        pane_id: "pane_1".into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        label: None,
        cwd: None,
        foreground_cwd: None,
        focused: false,
        right_click_passthrough: false,
    });
    shell.set_endpoint_snapshot(&remote, Box::new(snapshot));
    endpoints.set_surface_active(&ClientEndpointId::Local, false);
    endpoints.set_surface_active(&remote, true);
    assert!(endpoints.set_active(&remote));
    assert!(shell.activate_endpoint_projection(&remote));
    let mut pending = Some(
        PendingEndpointActivation::begin(
            &shell,
            &mut endpoints,
            ClientEndpointId::Local,
            None,
            resize(),
            80,
            Instant::now(),
        )
        .unwrap(),
    );
    assert!(
        !endpoints.active_surface_available(),
        "source is inactive during handoff"
    );
    let outcome = shell.activate_system_notification(ClientSystemNotificationTarget {
        endpoint_id: remote.clone(),
        boot_id: "remote-boot".into(),
        pane_id: "pane_1".into(),
    });
    let mut commands = EndpointCommands::default();
    let mut scheduled = None;
    dispatch_client_shell_actions(
        outcome.actions,
        &mut commands,
        &mut endpoints,
        Some(&mut shell),
        &mut Vec::new(),
        &mut scheduled,
    )
    .unwrap();
    let ClientLoopEvent::ActivateEndpoint {
        endpoint_id,
        target,
        force,
    } = scheduled.expect("click must reach the handoff manager")
    else {
        panic!("activation expected");
    };
    let mut state = ClientState::test_new();
    state.shell = Some(shell);
    begin_endpoint_activation(
        &mut state,
        &mut endpoints,
        &mut commands,
        &mut pending,
        &mut 81,
        endpoint_id,
        target,
        force,
        Instant::now(),
        &mut None,
    )
    .unwrap();
    assert_eq!(
        pending.unwrap().successor,
        Some(EndpointActivationIntent {
            endpoint_id: remote,
            target: Some(ClientEndpointFocusTarget::Notification {
                pane_id: "pane_1".into(),
                boot_id: "remote-boot".into()
            }),
        })
    );
}

/// A selection made while the target is being released for a rollback only
/// replaces the intent to resume. The release is still awaited: the target
/// has not failed to acknowledge it, so its connection is kept.
#[test]
fn a_selection_made_while_the_target_is_released_keeps_the_targets_connection() {
    let (shell, mut endpoints, _local_sent, _remote_sent) = shell_and_registry();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        endpoint(),
        None,
        resize(),
        24,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:24:off",
        &surface_success("client-shell-surface:24:off", false, 1),
        &mut endpoints,
    );

    for pane_id in ["pane_1", "pane_2"] {
        assert_eq!(
            activation.supersede(
                ClientEndpointId::Local,
                Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                    pane_id.into()
                )),
                &mut endpoints,
            ),
            ActivationRollback::Pending
        );
        assert!(matches!(
            activation.phase,
            ActivationPhase::ReleasingTargetForRollback { .. }
        ));
    }
    assert!(endpoints.take_failures().is_empty());
    assert_eq!(
        activation.successor,
        Some(EndpointActivationIntent {
            endpoint_id: ClientEndpointId::Local,
            target: Some(crate::client::shell::ClientEndpointFocusTarget::Pane(
                "pane_2".into()
            )),
        })
    );
}

#[test]
fn unacknowledged_target_release_closes_target_before_restoring_source() {
    let (shell, mut endpoints, local_sent, _remote_sent) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        None,
        resize(),
        24,
        Instant::now(),
    )
    .unwrap();
    let _ = activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:24:off",
        &surface_success("client-shell-surface:24:off", false, 1),
        &mut endpoints,
    );
    assert_eq!(
        activation.rollback(&mut endpoints, "target activation timed out".into(), false),
        ActivationRollback::Pending
    );
    assert_eq!(
        activation.rollback(&mut endpoints, "target release timed out".into(), false),
        ActivationRollback::Pending
    );
    assert!(endpoints.connection(&target).is_none());
    let failures = endpoints.take_failures();
    assert_eq!(
        failures.len(),
        1,
        "rollback revocation must reach the reconnect owner"
    );
    assert_eq!(failures[0].endpoint_id, target);
    assert_eq!(failures[0].generation, 7);
    assert_eq!(failures[0].kind, std::io::ErrorKind::TimedOut);
    assert_eq!(
        local_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
}

#[test]
fn target_loss_at_activation_deadline_restores_source_before_timeout() {
    let (shell, mut endpoints, local_sent, _) = shell_and_registry();
    let target = endpoint();
    let mut activation = PendingEndpointActivation::begin(
        &shell,
        &mut endpoints,
        target.clone(),
        None,
        resize(),
        30,
        Instant::now(),
    )
    .unwrap();
    activation.receive_response(
        &ClientEndpointId::Local,
        1,
        "client-shell-surface:30:off",
        &surface_success("client-shell-surface:30:off", false, 1),
        &mut endpoints,
    );
    let now = Instant::now();
    activation.deadline = now;
    assert!(activation.expired(now));
    endpoints.fail(&target, std::io::ErrorKind::UnexpectedEof.into());
    // Match the client timer: apply transport failures before checking phase expiry.
    for failure in endpoints.take_failures() {
        assert_eq!(
            activation.endpoint_disconnected(&mut endpoints, &failure.endpoint_id, failure.message),
            ActivationRollback::Pending
        );
    }
    assert!(matches!(
        activation.phase,
        ActivationPhase::RestoringSource { .. }
    ));
    assert!(!activation.expired(now));
    assert_eq!(
        local_sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(surface_set_active)
            .collect::<Vec<_>>(),
        vec![false, true]
    );
}

#[test]
fn losing_local_during_handoff_does_not_revoke_the_healthy_target() {
    for source_released in [false, true] {
        let (shell, mut endpoints, _local_sent, remote_sent) = shell_and_registry();
        let target = endpoint();
        let mut activation = PendingEndpointActivation::begin(
            &shell,
            &mut endpoints,
            target.clone(),
            None,
            resize(),
            29,
            Instant::now(),
        )
        .unwrap();
        if source_released {
            activation.receive_response(
                &ClientEndpointId::Local,
                1,
                "client-shell-surface:29:off",
                &surface_success("client-shell-surface:29:off", false, 1),
                &mut endpoints,
            );
        }
        if !source_released {
            activation.deadline = Instant::now() - Duration::from_millis(1);
        }
        endpoints.fail(
            &ClientEndpointId::Local,
            std::io::ErrorKind::BrokenPipe.into(),
        );
        assert_eq!(
            activation.endpoint_disconnected(
                &mut endpoints,
                &ClientEndpointId::Local,
                "Local stopped".into()
            ),
            ActivationRollback::Pending
        );
        assert!(!activation.source_available);
        assert!(
            !activation.expired(Instant::now()),
            "starting the healthy target must get a fresh deadline"
        );
        assert!(matches!(
            activation.phase,
            ActivationPhase::ActivatingTarget { .. }
        ));
        assert!(endpoints.connection(&target).is_some());
        assert_eq!(
            remote_sent
                .lock()
                .unwrap()
                .iter()
                .filter_map(surface_set_active)
                .collect::<Vec<_>>(),
            vec![true]
        );
    }
}

#[test]
fn resize_message_preserves_the_latest_surface_dimensions() {
    assert_eq!(
        resize_geometry(&resize()),
        Some(crate::protocol::ClientSurfaceSize { cols: 80, rows: 24 })
    );
}
