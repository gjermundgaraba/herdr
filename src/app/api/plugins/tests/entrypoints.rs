//! Plugin entrypoints: actions and panes share one id namespace, a key's
//! size applies only to a popup pane, and a pane opened by `plugin.invoke`
//! launches with the invoked target's context. An event hook names no
//! invoking client.

use super::*;

#[test]
fn link_rejects_action_and_pane_sharing_an_id() {
    let root = unique_temp_path("plugin-shared-entrypoint");
    write_manifest_content(
        &root,
        r#"
id = "example.shared-entrypoint"
name = "Shared Entrypoint"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos", "windows"]

[[actions]]
id = "palette"
title = "Palette action"
command = ["echo", "a"]

[[panes]]
id = "palette"
title = "Palette pane"
command = ["echo", "b"]
"#,
    );

    let result = load_plugin_manifest(&root.display().to_string(), true);
    assert!(matches!(result, Err(("duplicate_plugin_entrypoint_id", _))));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn plugin_invoke_validation_matrix() {
    type Setup = fn(&mut InstalledPluginInfo, &mut crate::api::schema::PluginInvokeParams);
    let root = unique_temp_path("plugin-invoke-validation");
    write_manifest(&root);
    let installed = load_plugin_manifest(&root.display().to_string(), true).unwrap();
    let cases: [(&str, Setup); 7] = [
        ("invalid_plugin_id", |_, params| {
            params.plugin_id = "bad plugin".into()
        }),
        ("plugin_not_found", |_, params| {
            params.plugin_id = "example.missing".into()
        }),
        ("invalid_plugin_entrypoint", |_, params| {
            params.entry_id = "bad entrypoint".into()
        }),
        ("plugin_entrypoint_not_found", |_, params| {
            params.entry_id = "missing".into()
        }),
        ("plugin_disabled", |plugin, _| plugin.enabled = false),
        ("plugin_manifest_unavailable", |plugin, _| {
            plugin.warnings.push("manifest unavailable: test".into())
        }),
        ("platform_unsupported", |plugin, _| {
            plugin.panes[0].platforms = Some(Vec::new())
        }),
    ];
    for (expected, setup) in cases {
        let mut app = test_app();
        let mut plugin = installed.clone();
        let mut params = invoke_params("example.worktree-bootstrap", "board");
        setup(&mut plugin, &mut params);
        app.state
            .installed_plugins
            .insert(plugin.plugin_id.clone(), plugin);
        let response = app.handle_plugin_invoke("invoke-validation".into(), params);
        let error: crate::api::schema::ErrorResponse =
            serde_json::from_str(&response).expect(&response);
        assert_eq!(error.error.code, expected);
        assert!(app.state.popup_pane.is_none());
        assert!(app.state.plugin_panes.is_empty());
        assert!(app.state.plugin_command_logs.is_empty());
    }
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn key_sizes_apply_only_to_popup_panes() {
    let root = unique_temp_path("plugin-pane-key-size");
    write_manifest_content(
        &root,
        r#"
id = "example.worktree-bootstrap"
name = "Worktree Bootstrap"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos", "windows"]

[[panes]]
id = "board"
title = "Board"
placement = "split"
command = ["sh", "-c", "exit 0"]
"#,
    );
    let installed = load_plugin_manifest(&root.display().to_string(), true).unwrap();
    let mut app = test_app();
    app.state.workspaces = vec![crate::workspace::Workspace::test_new("target")];
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 40);
    app.state
        .installed_plugins
        .insert(installed.plugin_id.clone(), installed);
    let mut binding = plugin_keybinding("example.worktree-bootstrap.board");
    binding.width = Some(crate::popup_size::PopupSize::Percent(80));
    binding.height = Some(crate::popup_size::PopupSize::Cells(12));
    app.invoke_plugin_from_keybind(&binding, None).unwrap();
    assert!(app.state.popup_pane.is_none());
    assert_eq!(app.state.plugin_panes.len(), 1);
    for (_, runtime) in app.terminal_runtimes.drain() {
        runtime.shutdown();
    }
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn invoked_pane_launches_with_the_invoked_context() {
    let root = unique_temp_path("plugin-invoke-pane-context");
    write_manifest_content(
        &root,
        r#"
id = "example.invoked"
name = "Invoked"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos", "windows"]

[[panes]]
id = "board"
title = "Board"
placement = "split"
command = ["sh", "-c", "printf '%s' \"$HERDR_PLUGIN_CONTEXT_JSON\" > context.json; : > ready"]

[[panes]]
id = "palette"
title = "Palette"
placement = "popup"
command = ["sh", "-c", "exit 0"]
"#,
    );
    let installed = load_plugin_manifest(&root.display().to_string(), true).unwrap();
    let mut app = test_app();
    app.state.workspaces = vec![
        crate::workspace::Workspace::test_new("active"),
        crate::workspace::Workspace::test_new("target"),
    ];
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 40);
    app.state
        .installed_plugins
        .insert(installed.plugin_id.clone(), installed);
    let target_pane = app.state.workspaces[1].tabs[0].root_pane;
    let target_id = app.public_pane_id(1, target_pane).unwrap();
    let workspace_id = app.public_workspace_id(1);

    let mut params = invoke_params("example.invoked", "board");
    params.pane_id = Some(target_id.clone());
    let response = app.handle_plugin_invoke("invoke-split".into(), params);
    let ResponseResult::PluginPaneOpened { plugin_pane } = response_result(&response) else {
        panic!("expected opened pane: {response}");
    };
    assert_eq!(plugin_pane.pane.workspace_id, workspace_id);
    assert!(app.fork.invoked_plugin_context.is_none());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !root.join("ready").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pane did not finish writing context");
    let context: PluginInvocationContext =
        serde_json::from_str(&std::fs::read_to_string(root.join("context.json")).unwrap()).unwrap();
    assert_eq!(context.workspace_id.as_deref(), Some(workspace_id.as_str()));
    assert_eq!(context.focused_pane_id.as_deref(), Some(target_id.as_str()));
    assert_eq!(context.correlation_id.as_deref(), Some("invoke-split"));

    // An open that fails after the context is set clears it as well.
    let palette = invoke_params("example.invoked", "palette");
    let response = app.handle_plugin_invoke("first-popup".into(), palette.clone());
    assert_eq!(response_result(&response), ResponseResult::Ok {});
    assert!(app.state.popup_pane.is_some());
    let response = app.handle_plugin_invoke("busy-popup".into(), palette);
    let error: crate::api::schema::ErrorResponse =
        serde_json::from_str(&response).expect(&response);
    assert_eq!(error.error.code, "ui_busy");
    assert!(app.fork.invoked_plugin_context.is_none());
    for (_, runtime) in app.terminal_runtimes.drain() {
        runtime.shutdown();
    }
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn event_hooks_name_no_invoking_client() {
    let mut app = test_app();
    let root = unique_temp_path("plugin-event-client");
    let capture = root.join("capture");
    write_manifest(&root);
    link_manifest(&mut app, &root);
    let plugin = app.state.installed_plugins["example.worktree-bootstrap"].clone();
    let context = app.current_plugin_context("event");
    app.fork.invoking_client_id = Some(7);
    let command = format!(
        "printf '%s\\n%s' \"${{HERDR_CLIENT_ID-unset}}\" \"$HERDR_PLUGIN_CONTEXT_JSON\" > {}",
        capture.display()
    );
    app.start_plugin_command(
        &plugin,
        None,
        Some("worktree.created".into()),
        vec!["sh".into(), "-c".into(), command],
        &context,
        Some("{}".into()),
    )
    .unwrap();
    let captured = read_capture_when_ready(&capture, || {
        app.drain_all_internal_events();
    });
    let (client_id, context) = captured.split_once('\n').unwrap();
    assert_eq!(client_id, "unset");
    let context: PluginInvocationContext = serde_json::from_str(context).unwrap();
    assert_eq!(context.client_id, None);
    let _ = std::fs::remove_dir_all(root);
}
