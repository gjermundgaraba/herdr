//! Plugin entrypoints: actions and panes share one id namespace, and a
//! plugin pane ignores sizes outside popups and takes its context from an
//! explicit target.

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

#[cfg(unix)]
#[tokio::test]
async fn pane_sizes_are_ignored_outside_popups_from_manifest_api_and_keybinding() {
    let root = unique_temp_path("plugin-pane-non-popup-size");
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
width = "80%"
height = 12
command = ["sh", "-c", "exit 0"]
"#,
    );
    let installed = load_plugin_manifest(&root.display().to_string(), true).unwrap();
    for from_keybinding in [false, true] {
        let mut app = test_app();
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("target")];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 40);
        app.state
            .installed_plugins
            .insert(installed.plugin_id.clone(), installed.clone());
        let width = Some(crate::popup_size::PopupSize::Percent(80));
        if from_keybinding {
            let mut binding = plugin_keybinding("example.worktree-bootstrap.board");
            binding.width = width;
            app.invoke_plugin_from_keybind(&binding, None).unwrap();
        } else {
            let mut params = pane_open_params();
            params.width = width;
            let response = app.handle_plugin_pane_open("pane-open-size".into(), params);
            assert!(matches!(
                response_result(&response),
                ResponseResult::PluginPaneOpened { .. }
            ));
        }
        assert!(app.state.popup_pane.is_none());
        assert_eq!(app.state.plugin_panes.len(), 1);
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn plugin_pane_api_context_follows_explicit_split_and_workspace_targets() {
    let root = unique_temp_path("plugin-pane-explicit-target");
    write_manifest(&root);
    let installed = load_plugin_manifest(&root.display().to_string(), true).unwrap();
    for placement in [PluginPanePlacement::Split, PluginPanePlacement::Tab] {
        let mut app = test_app();
        app.state.workspaces = vec![
            crate::workspace::Workspace::test_new("active"),
            crate::workspace::Workspace::test_new("explicit-target"),
        ];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        let target_pane = app.state.workspaces[1].tabs[0].root_pane;
        let target_id = app.public_pane_id(1, target_pane).unwrap();
        let workspace_id = app.public_workspace_id(1);
        let mut params = pane_open_params();
        params.placement = Some(placement);
        if placement == PluginPanePlacement::Split {
            params.target_pane_id = Some(target_id.clone());
        } else {
            params.workspace_id = Some(workspace_id.clone());
        }
        let mut plugin = installed.clone();
        plugin.panes[0].command = vec![
            "sh".into(),
            "-c".into(),
            "printf '%s' \"$HERDR_PLUGIN_CONTEXT_JSON\" > context.json; : > ready".into(),
        ];
        app.state
            .installed_plugins
            .insert(plugin.plugin_id.clone(), plugin);
        let response = app.handle_plugin_pane_open("explicit-target".into(), params);
        let ResponseResult::PluginPaneOpened { plugin_pane } = response_result(&response) else {
            panic!("expected opened pane: {response}");
        };
        assert_eq!(plugin_pane.pane.workspace_id, workspace_id);
        assert_eq!(app.state.active, Some(0));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !root.join("ready").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("pane did not finish writing context");
        let context: PluginInvocationContext =
            serde_json::from_str(&std::fs::read_to_string(root.join("context.json")).unwrap())
                .unwrap();
        assert_eq!(context.workspace_id.as_deref(), Some(workspace_id.as_str()));
        assert_eq!(context.focused_pane_id.as_deref(), Some(target_id.as_str()));
        assert_eq!(context.invocation_source.as_deref(), Some("api"));
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
        std::fs::remove_file(root.join("ready")).unwrap();
    }
    let _ = std::fs::remove_dir_all(root);
}
