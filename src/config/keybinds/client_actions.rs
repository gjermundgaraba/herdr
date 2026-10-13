//! Actions handled by the TUI client itself. They come from the local config
//! even when an endpoint's keybinding profile is in effect, and win a key that
//! profile also binds (`input::keybindings::resolve_non_indexed_action`).
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/keymap/daemon/client_actions.rs` (see "Sibling
//! fork: Herdr GPUI" in `FORK.md`).

use super::Keybinds;

impl Keybinds {
    pub(crate) fn copy_client_actions(&mut self, from: &Keybinds) {
        self.agent_picker = from.agent_picker.clone();
        self.workspace_list = from.workspace_list.clone();
        self.history_back = from.history_back.clone();
        self.history_forward = from.history_forward.clone();
        self.mark_unread = from.mark_unread.clone();
    }
}
