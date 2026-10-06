//! `type = "plugin"` key commands. The command names one entrypoint as
//! `plugin_id.entrypoint`; actions run as plugin commands and panes open
//! directly, both with the originating pane and selection as context.

use super::entrypoint::PluginEntrypoint;
use super::panes::PluginPaneInvocation;
use crate::api::schema::PluginPaneOpenParams;
use crate::app::App;
use crate::config::CustomCommandKeybind;

impl App {
    pub(crate) fn invoke_plugin_from_keybind(
        &mut self,
        binding: &CustomCommandKeybind,
        selected_text: Option<String>,
    ) -> Result<(), String> {
        self.refresh_installed_plugins()
            .map_err(|err| format!("plugin_registry_load_failed: {err}"))?;
        let result = self
            .resolve_qualified_plugin_entrypoint(&binding.command)
            .and_then(|(plugin, entrypoint)| match entrypoint {
                PluginEntrypoint::Action(action) => {
                    let mut context = self.current_plugin_context("keybinding");
                    context.invocation_source = Some("keybinding".to_owned());
                    context.selected_text = selected_text;
                    self.start_plugin_command(
                        &plugin,
                        Some(action.action_id),
                        None,
                        action.command,
                        &context,
                        None,
                    )
                    .map(|_| ())
                    .map_err(|(code, message)| super::entrypoint::plugin_error(code, message))
                }
                PluginEntrypoint::Pane(pane) => self
                    .open_plugin_pane(
                        plugin,
                        pane,
                        PluginPaneOpenParams {
                            plugin_id: String::new(),
                            entrypoint: String::new(),
                            placement: None,
                            width: binding.width,
                            height: binding.height,
                            workspace_id: None,
                            target_pane_id: None,
                            direction: None,
                            cwd: None,
                            focus: true,
                            env: Default::default(),
                        },
                        PluginPaneInvocation::Keybinding { selected_text },
                    )
                    .map(|_| ()),
            });
        result.map_err(|error| format!("{}: {}", error.code, error.message))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::plugin_keybinding as binding;
    use super::*;
    use crate::api::schema::{
        CommandInvokeParams, ErrorResponse, InstalledPluginInfo, SuccessResponse,
    };
    use crate::popup_size::PopupSize;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = super::super::tests::unique_temp_path("plugin-keybinding");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join("herdr-plugin.toml"),
                r#"
id = "example.keybound"
name = "Keybound"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos", "windows"]

[[actions]]
id = "run"
title = "Run"
command = ["sh", "-c", "true"]

[[panes]]
id = "palette"
title = "Palette"
placement = "popup"
width = "70%"
height = 18
command = ["./pane.sh"]
"#,
            )
            .unwrap();
            Self { root }
        }

        fn plugin(&self) -> InstalledPluginInfo {
            super::super::load_plugin_manifest(&self.root.display().to_string(), true).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn test_app(fixture: &Fixture) -> App {
        let mut app = super::super::tests::test_app();
        app.state.workspaces = vec![
            crate::workspace::Workspace::test_new("server-active"),
            crate::workspace::Workspace::test_new("client-target"),
        ];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 100, 40);
        let plugin = fixture.plugin();
        app.state
            .installed_plugins
            .insert(plugin.plugin_id.clone(), plugin);
        app
    }

    fn install(app: &mut App, binding: CustomCommandKeybind) -> String {
        app.endpoint_commands =
            crate::app::custom_commands::EndpointCommandRegistry::new(&[binding]);
        app.client_shell_command_manifest()[0].command_id.clone()
    }

    fn assert_no_action_launcher(app: &App) {
        assert!(app.state.plugin_command_logs.is_empty());
        assert_eq!(app.state.plugin_commands_in_flight, 0);
        assert!(app.detached_process_children.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pane_binding_opens_manifest_popup_with_client_context() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = Fixture::new();
        let script = fixture.root.join("pane.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
printf '%s' "$HERDR_PLUGIN_CONTEXT_JSON" > context.json
: > ready
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut app = test_app(&fixture);
        let target_pane = app.state.workspaces[1].tabs[0].root_pane;
        let target_terminal = app.state.workspaces[1]
            .terminal_id(target_pane)
            .unwrap()
            .clone();
        app.terminal_runtimes.insert(
            target_terminal,
            crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 24, b"selected text\n"),
        );
        let target_id = app.public_pane_id(1, target_pane).unwrap();
        let workspace_id = app.public_workspace_id(1);
        let tab_id = app.public_tab_id(1, 0).unwrap();
        let command_id = install(&mut app, binding("example.keybound.palette"));
        let response = app.handle_command_invoke(
            "pane-binding".into(),
            CommandInvokeParams {
                command_id,
                workspace_id: Some(workspace_id.clone()),
                tab_id: Some(tab_id.clone()),
                pane_id: Some(target_id.clone()),
                selection: Some(crate::api::schema::PaneSelectionReadParams {
                    pane_id: target_id.clone(),
                    anchor: crate::api::schema::PaneTextPoint { row: 0, col: 0 },
                    cursor: crate::api::schema::PaneTextPoint { row: 0, col: 7 },
                    content_revision: None,
                }),
            },
        );
        let success: SuccessResponse = serde_json::from_str(&response).expect(&response);
        assert_eq!(success.result, crate::api::schema::ResponseResult::Ok {});
        let popup = app.state.popup_pane.as_ref().unwrap();
        assert_eq!(popup.width, Some(PopupSize::Percent(70)));
        assert_eq!(popup.height, Some(PopupSize::Cells(18)));
        assert_eq!(app.state.active, Some(1));
        assert_eq!(app.state.workspaces[1].focused_pane_id(), Some(target_pane));
        assert_no_action_launcher(&app);

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !fixture.root.join("ready").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("pane command did not finish writing its launch environment");
        let context: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.root.join("context.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(context["workspace_id"], workspace_id);
        assert_eq!(context["tab_id"], tab_id);
        assert_eq!(context["focused_pane_id"], target_id);
        assert_eq!(context["selected_text"], "selected");
        assert_eq!(context["invocation_source"], "keybinding");
        assert!(app.close_popup_pane());

        let mut resized = binding("example.keybound.palette");
        resized.width = Some(PopupSize::Cells(60));
        resized.height = Some(PopupSize::Percent(50));
        app.execute_custom_command_binding(&resized, None).unwrap();
        let popup = app.state.popup_pane.as_ref().unwrap();
        assert_eq!(popup.width, resized.width);
        assert_eq!(popup.height, resized.height);
        assert_no_action_launcher(&app);
        assert!(app.close_popup_pane());
    }

    #[test]
    fn popup_pane_bindings_advertise_as_popup_commands() {
        let fixture = Fixture::new();
        let mut app = test_app(&fixture);
        for (target, expected) in [
            (
                "example.keybound.palette",
                crate::protocol::ClientShellCommandAction::Popup,
            ),
            (
                "example.keybound.run",
                crate::protocol::ClientShellCommandAction::PluginAction,
            ),
            (
                "example.keybound.missing",
                crate::protocol::ClientShellCommandAction::PluginAction,
            ),
        ] {
            install(&mut app, binding(target));
            assert_eq!(
                app.client_shell_command_manifest()[0].action,
                expected,
                "{target}"
            );
        }
    }

    #[test]
    fn plugin_binding_reports_malformed_and_missing_targets() {
        let fixture = Fixture::new();
        let mut app = test_app(&fixture);
        for (command, expected) in [
            ("palette", "qualified plugin_id.entrypoint"),
            ("example.keybound.missing", "plugin_entrypoint_not_found"),
        ] {
            let error = app
                .execute_custom_command_binding(&binding(command), None)
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert!(app.state.popup_pane.is_none());
            assert_no_action_launcher(&app);
        }
    }

    #[cfg(unix)]
    #[test]
    fn action_binding_runs_without_opening_a_pane() {
        let fixture = Fixture::new();
        let mut app = test_app(&fixture);
        app.execute_custom_command_binding(&binding("example.keybound.run"), None)
            .unwrap();
        assert_eq!(app.state.plugin_commands_in_flight, 1);
        assert!(app.state.popup_pane.is_none());
        assert!(app.state.plugin_panes.is_empty());
    }

    #[test]
    fn plugin_binding_rejects_mismatched_client_target_before_launching() {
        let fixture = Fixture::new();
        let mut app = test_app(&fixture);
        let command_id = install(&mut app, binding("example.keybound.palette"));
        let response = app.handle_command_invoke(
            "wrong-target".into(),
            CommandInvokeParams {
                command_id,
                workspace_id: Some(app.public_workspace_id(0)),
                tab_id: None,
                pane_id: app.public_pane_id(1, app.state.workspaces[1].tabs[0].root_pane),
                selection: None,
            },
        );
        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "command_target_mismatch");
        assert_eq!(app.state.active, Some(0));
        assert!(app.state.popup_pane.is_none());
        assert_no_action_launcher(&app);
    }
}
