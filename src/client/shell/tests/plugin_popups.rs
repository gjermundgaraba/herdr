//! Plugin key commands hold input for a popup only when the host advertises
//! one.

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
            action: crate::config::CustomCommandAction::Plugin,
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
