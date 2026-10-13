//! Plugin key commands: they hold input for a popup only when the host
//! advertises one, and real keypresses reach the host's opaque command ids.

use super::*;

#[test]
fn plugin_commands_hold_input_only_when_advertised_as_popups() {
    for (advertised, guarded) in [
        (crate::protocol::ClientShellCommandAction::Popup, true),
        (
            crate::protocol::ClientShellCommandAction::PluginAction,
            false,
        ),
    ] {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        // A locally configured plugin binding matches the server's command by
        // its keys, whichever category the server advertises.
        let binding = crate::config::CustomCommandKeybind {
            bindings: crate::config::ActionKeybinds::prefix("t"),
            label: "prefix+t".into(),
            command: "example.plugin.palette".into(),
            action: crate::config::CustomCommandAction::PluginAction,
            description: None,
            width: None,
            height: None,
        };
        let mut projection = snapshot();
        projection
            .commands
            .push(crate::protocol::ClientShellCommand {
                command_id: "cmd_plugin".into(),
                binding_label: binding.label.clone(),
                binding_labels: binding.bindings.labels(),
                action: advertised,
                description: None,
            });
        state.set_snapshot(Box::new(projection));
        state.set_pane_surface(surface());

        let mut invoke = ClientShellInput::default();
        state.record_binding(crate::input::KeybindMatch::Command(binding), &mut invoke);
        assert!(matches!(
            &invoke.actions[..],
            [ClientShellAction::Endpoint { request, .. }] if matches!(
                &request.method,
                crate::api::schema::Method::CommandInvoke(params) if params.command_id == "cmd_plugin"
            )
        ));
        assert_eq!(state.popup_pending, guarded, "{advertised:?}");
        assert_eq!(
            state.handle_input_bytes(b"x").requests.is_empty(),
            guarded,
            "{advertised:?}"
        );
        state.set_pane_surface(surface_with_popup());
        assert!(!state.popup_pending, "{advertised:?}");
    }
}

#[test]
fn snapshot_plugin_commands_resolve_opaque_ids_through_actual_keypresses() {
    for source in [
        ClientShellKeybindingSource::Local,
        ClientShellKeybindingSource::Endpoint,
    ] {
        for label in ["prefix+z", "ctrl+g"] {
            let mut config = Config::default();
            config
                .keys
                .command
                .push(crate::config::CommandKeybindConfig {
                    key: crate::config::BindingConfig::One(label.into()),
                    command: "example.plugin.palette".into(),
                    action_type: crate::config::CommandKeybindType::PluginAction,
                    ..Default::default()
                });
            let mut state = ClientShellState::new(
                ClientShellConfig::from_config(&config).with_keybinding_source(source),
            );
            let mut projection = snapshot();
            projection.server_keybindings_toml = config.local_keybindings_profile_toml().ok();
            projection
                .commands
                .push(crate::protocol::ClientShellCommand {
                    // Endpoint IDs have no authored plugin-target syntax requirement.
                    command_id: "opaque/server-command".into(),
                    binding_label: label.into(),
                    binding_labels: vec![label.into()],
                    action: crate::config::CustomCommandAction::PluginAction.into(),
                    description: None,
                });
            state.set_snapshot(Box::new(projection));
            let mut pane_surface = surface();
            pane_surface.panes[0].content_revision = 42;
            state.set_pane_surface(pane_surface);
            let mut selection =
                crate::selection::Selection::absolute_range("pane_1".to_owned(), (2, 3), (4, 5));
            assert!(selection.finish());
            state.selection = Some(selection);

            let outcome = if label == "prefix+z" {
                assert!(state.handle_input_bytes(&[0x02]).requests.is_empty());
                assert!(
                    state.selection.is_some(),
                    "prefix entry must preserve selection"
                );
                state.handle_input_bytes(b"z")
            } else {
                state.handle_input_bytes(&[0x07])
            };
            let [ClientShellAction::Endpoint { request, .. }] = &outcome.actions[..] else {
                panic!(
                    "expected snapshot command from actual keypress: {source:?}, {label}: {:?}",
                    outcome.actions
                );
            };
            let crate::api::schema::Method::CommandInvoke(params) = &request.method else {
                panic!(
                    "expected command.invoke: {source:?}, {label}: {:?}",
                    request.method
                );
            };
            assert_eq!(params.command_id, "opaque/server-command");
            assert_eq!(
                params.selection,
                Some(crate::api::schema::PaneSelectionReadParams {
                    pane_id: "pane_1".into(),
                    anchor: crate::api::schema::PaneTextPoint { row: 2, col: 3 },
                    cursor: crate::api::schema::PaneTextPoint { row: 4, col: 5 },
                    content_revision: Some(42),
                })
            );
            // Ordinary terminal input must still clear the retained selection.
            let typing = state.handle_input_bytes(b"a");
            assert!(state.selection.is_none());
            assert!(matches!(
                &typing.requests[..],
                [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1"
            ));
        }
    }
}
