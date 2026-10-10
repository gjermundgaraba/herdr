//! When a pane counts as presented, for `agent.mark_unread`. The hold itself
//! is session state (`app/unread_hold.rs`); presence depends on the attached
//! clients, so the server settles the holds after every event that can move
//! a client.
//!
//! A client presents the focused pane of the tab it is on, while its shell
//! surface is active. An armed hold is restored once the client that marked
//! it presents another pane, switches endpoint, or detaches. A restored hold
//! ends when a client arrives at the pane: it starts presenting it, or its
//! window regains focus while it does.

use std::collections::HashMap;

use super::HeadlessServer;
use crate::layout::PaneId;
use crate::pane::UnreadHold;

/// The pane each client presented, with its window not known to be
/// unfocused, when the holds were last settled. Kept only while holds exist.
#[derive(Default)]
pub(super) struct UnreadPresence(HashMap<u64, PaneId>);

impl HeadlessServer {
    /// Restores and ends unread holds according to what clients now present.
    /// Returns whether an agent's status changed.
    pub(super) fn settle_unread_holds(&mut self) -> bool {
        if !self.app.state.unread_holds_pending {
            return false;
        }
        let holds = self.app.state.unread_holds();
        if holds.is_empty() {
            self.app.state.unread_holds_pending = false;
            self.unread_presence.0.clear();
            return false;
        }
        let presenting = self
            .shell_focus_targets()
            .into_iter()
            .filter_map(|(client_id, target)| Some((client_id, target?.pane_id)))
            .collect::<HashMap<_, _>>();
        let attentive = presenting
            .iter()
            .filter(|(client_id, _)| {
                self.clients
                    .get(client_id)
                    .is_some_and(|client| client.outer_terminal_focus != Some(false))
            })
            .map(|(&client_id, &pane_id)| (client_id, pane_id))
            .collect::<HashMap<_, _>>();

        let mut changed = false;
        for (ws_idx, pane_id, hold) in holds {
            match hold {
                UnreadHold::Armed { client_id } => {
                    let still_presented = match client_id {
                        Some(client_id) => presenting.get(&client_id) == Some(&pane_id),
                        None => presenting.values().any(|presented| *presented == pane_id),
                    };
                    if !still_presented {
                        changed |= self.app.restore_unread_hold(ws_idx, pane_id);
                    }
                }
                UnreadHold::Restored => {
                    let arrived = attentive.iter().any(|(client_id, presented)| {
                        *presented == pane_id
                            && self.unread_presence.0.get(client_id) != Some(&pane_id)
                    });
                    if arrived {
                        changed |= self.app.state.clear_unread_hold(ws_idx, pane_id);
                    }
                }
            }
        }
        if self.app.state.unread_holds().is_empty() {
            self.app.state.unread_holds_pending = false;
            self.unread_presence.0.clear();
        } else {
            self.unread_presence.0 = attentive;
        }
        changed
    }
}
