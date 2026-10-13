//! A plugin started from a client's key command receives that client's id.
#![cfg(unix)]

use super::*;

#[tokio::test]
async fn plugin_key_commands_from_a_client_receive_that_client_id() {
    let root = std::env::temp_dir().join(format!(
        "herdr-plugin-client-origin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("herdr-plugin.toml"),
        r#"
id = "example.client-origin"
name = "Client origin"
version = "0.1.0"
min_herdr_version = "0.6.10"
platforms = ["linux", "macos", "windows"]

[[actions]]
id = "record"
title = "Record"
command = ["sh", "-c", "printf '%s' \"$HERDR_CLIENT_ID\" > client.tmp && mv client.tmp client"]
"#,
    )
    .unwrap();
    let plugin = crate::app::load_plugin_manifest(&root.display().to_string(), true).unwrap();
    let mut server = test_headless_server();
    server.app.state.workspaces = vec![crate::workspace::Workspace::test_new("origin")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server
        .app
        .state
        .installed_plugins
        .insert(plugin.plugin_id.clone(), plugin);
    // A client reaches a plugin through a `type = "plugin"` key command.
    server
        .app
        .set_custom_commands_for_test(&[crate::config::CustomCommandKeybind {
            bindings: crate::config::ActionKeybinds::prefix("z"),
            label: "prefix+z".into(),
            command: "example.client-origin.record".into(),
            action: crate::config::CustomCommandAction::Plugin,
            description: None,
            width: None,
            height: None,
        }]);
    let command_id = server.app.client_shell_command_manifest()[0]
        .command_id
        .clone();
    let (_control, _) = connect_matching_test_shell(&mut server, 61);

    let (respond_to, _response_rx) = std::sync::mpsc::channel();
    server.handle_client_shell_api_request(
        61,
        crate::api::ApiRequestMessage {
            request: crate::api::schema::Request {
                id: "record-origin".into(),
                method: crate::api::schema::Method::CommandInvoke(
                    crate::api::schema::CommandInvokeParams {
                        command_id,
                        workspace_id: None,
                        tab_id: None,
                        pane_id: None,
                        selection: None,
                    },
                ),
            },
            respond_to,
            response_write_complete: None,
        },
    );
    assert_eq!(server.app.invoking_client_id, None);
    let recorded = root.join("client");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !recorded.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("plugin action did not record its client id");
    assert_eq!(std::fs::read_to_string(&recorded).unwrap(), "61");
    shutdown_test_runtimes(&mut server);
    let _ = std::fs::remove_dir_all(root);
}
