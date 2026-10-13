//! The `[keys]` actions the TUI handles itself: the agent and workspace
//! lists, Back and Forward, and `mark_unread`. Their bindings stay the local
//! config's under any endpoint's keybinding profile
//! (`crate::config::keybinds::client_actions`).

use super::*;
use crate::input::KeybindAction;

impl ClientShellState {
    /// Runs a client-side action; false for any other action.
    pub(super) fn client_action_key(
        &mut self,
        action: KeybindAction,
        outcome: &mut ClientShellInput,
    ) -> bool {
        match action {
            KeybindAction::AgentPicker => self.open_navigator(NavigatorLayout::Agents),
            KeybindAction::WorkspaceList => self.open_navigator(NavigatorLayout::Workspaces),
            KeybindAction::MarkUnread => {
                self.mark_focused_pane_unread(outcome);
                return true;
            }
            _ => return self.history_key(action, outcome),
        }
        outcome.repaint = true;
        true
    }
}

impl ClientShellConfig {
    /// Under an endpoint's profile, which stays in effect on reload, rebinds
    /// the client's own actions from the reloaded local config, as when the
    /// client started.
    pub(super) fn reload_client_actions(&mut self, config: &Config) {
        if self.keybinding_source != ClientShellKeybindingSource::Endpoint {
            return;
        }
        self.local_client_actions = config.keybinds();
        self.keybinds
            .keybinds
            .copy_client_actions(&self.local_client_actions);
    }
}
