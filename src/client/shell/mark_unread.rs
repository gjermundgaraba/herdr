//! The `mark_unread` key: asks the daemon that owns the focused pane to hold
//! its agent's acknowledged completion (`agent.mark_unread`). The daemon keeps
//! the hold and decides when the badge comes back and when it clears, so this
//! client records nothing: it shows what the next snapshots say, and confirms
//! an accepted request, since nothing on screen changes until the user leaves
//! the pane.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/mark_unread.rs` (see "Sibling fork: Herdr GPUI" in
//! `FORK.md`).

use super::*;

impl ClientShellState {
    pub(super) fn mark_focused_pane_unread(&mut self, outcome: &mut ClientShellInput) {
        let Some(pane_id) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone())
        else {
            outcome.repaint |= self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                "mark_unread",
                "Nothing to mark",
                "No pane is focused.",
            );
            return;
        };
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::AgentMarkUnread(crate::api::schema::PaneTarget { pane_id }),
            PendingEndpointKind::MarkUnread,
            outcome,
        );
    }

    /// The daemon's answer to `agent.mark_unread`, as
    /// [`Self::handle_endpoint_result`] returns it. A refusal has already
    /// been shown as a rejected request.
    pub(super) fn mark_unread_answered(
        &mut self,
        accepted: bool,
    ) -> (bool, Vec<ClientShellAction>) {
        if !accepted {
            return (true, Vec::new());
        }
        let repaint = self.push_endpoint_notice(
            ClientEndpointNoticeKind::Rejected,
            "mark_unread",
            "Marked unread",
            "The badge returns and the agent moves back in the queue when you leave this pane.",
        );
        (repaint, Vec::new())
    }
}
