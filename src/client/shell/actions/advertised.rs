//! How the daemon advertises a custom command binding. Herdr GPUI's palette
//! (`crates/herdr-gpui/src/palette.rs`) takes the advertised kind as it is.

use crate::protocol::ClientShellCommandAction;

/// Whether a command the daemon advertises as `advertised` serves a local
/// binding of kind `local`. A plugin binding whose target opens a popup pane
/// advertises as a popup, so the client holds input until the popup arrives
/// (`App::client_shell_command_action`).
pub(super) fn advertised_as(
    advertised: ClientShellCommandAction,
    local: ClientShellCommandAction,
) -> bool {
    advertised == local
        || (local == ClientShellCommandAction::PluginAction
            && advertised == ClientShellCommandAction::Popup)
}
