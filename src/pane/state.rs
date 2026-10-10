use crate::terminal::TerminalId;

/// Viewport state for a pane.
///
/// Terminal identity, cwd, labels, and agent metadata live in TerminalState.
pub struct PaneState {
    pub attached_terminal_id: TerminalId,
    /// Whether the user has seen this pane since its last state change to Idle.
    /// False = "Done" (agent finished while user was in another workspace).
    pub seen: bool,
    /// Whether unmodified right-click gestures should be forwarded to the pane application.
    pub right_click_passthrough: bool,
    /// Set by `agent.mark_unread` until the pane is presented again.
    pub unread_hold: Option<UnreadHold>,
}

/// A completion the user asked to see again (`agent.mark_unread`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadHold {
    /// Marked while the pane is presented. The completion comes back once
    /// `client_id` stops presenting the pane.
    Armed { client_id: Option<u64> },
    /// The completion is back: the pane stays unseen until a client presents
    /// it again, so marks that cover a whole tab pass over it.
    Restored,
}

impl PaneState {
    /// Marks the pane seen, unless a restored completion is held on it.
    pub fn mark_seen(&mut self) -> bool {
        if self.seen || self.unread_hold == Some(UnreadHold::Restored) {
            return false;
        }
        self.seen = true;
        true
    }
}

impl PaneState {
    pub fn new(attached_terminal_id: TerminalId) -> Self {
        Self {
            attached_terminal_id,
            seen: true,
            right_click_passthrough: false,
            unread_hold: None,
        }
    }
}
