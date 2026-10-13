//! What the fork keeps per server beside upstream's state: open pickers and
//! jumps, client reports, and the presence unread holds settle against.

use std::collections::HashMap;

use super::{client_reports, jumps, picks, unread_holds, ClientConnection, HeadlessServer};

#[derive(Default)]
pub(super) struct ForkState {
    /// Open `ui.pick` pickers by pick id, each waiting on its API caller.
    pub(super) pending_picks: HashMap<String, picks::PendingPick>,
    /// Latest `client.report` per client.
    pub(super) client_reports: client_reports::ClientReports,
    /// Open `client.jump` requests, each waiting on its client's answer.
    pub(super) client_jumps: jumps::ClientJumps,
    /// Client presence the unread holds were last settled against.
    pub(super) unread_presence: unread_holds::UnreadPresence,
}

impl HeadlessServer {
    /// Cancels a removed client's pickers and jump, and drops its report.
    pub(super) fn forget_fork_client(
        &mut self,
        client_id: u64,
        removed: Option<&ClientConnection>,
    ) {
        self.cancel_client_picks(client_id);
        self.cancel_client_jump(client_id);
        self.forget_client_report(
            client_id,
            removed.is_some_and(|client| client.is_shell_client()),
        );
    }
}
