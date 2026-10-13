//! The unread hold a pane carries, as pane data. `app/unread_hold.rs` owns
//! its transitions for `agent.mark_unread`.

use super::PaneState;

/// A completion the user asked to see again (`agent.mark_unread`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnreadHold {
    /// Marked while the pane is presented. The completion comes back once
    /// `client_id` stops presenting the pane.
    Armed { client_id: Option<u64> },
    /// The completion is back: the pane stays unseen until a client presents
    /// it again, so marks that cover a whole tab pass over it.
    Restored,
}

impl PaneState {
    /// Marks the pane seen, unless a restored completion is held on it.
    pub(crate) fn mark_seen(&mut self) -> bool {
        if self.seen || self.unread_hold == Some(UnreadHold::Restored) {
            return false;
        }
        self.seen = true;
        true
    }
}
