//! What the fork keeps per server beside upstream's state: open pickers and
//! the presence unread holds settle against.

use std::collections::HashMap;

use super::picks;

#[derive(Default)]
pub(super) struct ForkState {
    /// Open `ui.pick` pickers by pick id, each waiting on its API caller.
    pub(super) pending_picks: HashMap<String, picks::PendingPick>,
    /// The pane each client presented, with its window not known to be
    /// unfocused, when the unread holds were last settled. Kept only while
    /// holds exist.
    pub(super) unread_presence: HashMap<u64, crate::layout::PaneId>,
}
