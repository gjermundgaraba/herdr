//! `ui.pick` pickers a daemon asks this client to show. Every picker is
//! answered with `ui.pick.resolve` on the connection that opened it: the
//! user's choice, or `cancelled` when the picker goes away by any other path,
//! such as a switch to another endpoint or another overlay replacing it, or
//! when the choice is created text too long to send. The waiting plugin
//! therefore never hangs on a picker this client dropped.
//!
//! Only the presented endpoint shows pickers. A picker from another endpoint
//! is cancelled as it arrives.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/picker.rs` and `picker/` (see "Sibling fork: Herdr
//! GPUI" in `FORK.md`).

use crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::api::schema::UiPickOutcome;
use crate::protocol::endpoint::{EndpointPickClose, EndpointPickOpen};

/// The picker this client shows, by the connection that opened it.
#[derive(Debug)]
struct OpenPick {
    origin: ControlOrigin,
    pick_id: String,
}

/// A picker answer ready to send.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPick {
    pub(crate) origin: ControlOrigin,
    pub(crate) pick_id: String,
    pub(crate) outcome: UiPickOutcome,
}

/// The open picker, and answers not yet collected.
#[derive(Debug, Default)]
pub(crate) struct ClientPicks {
    open: Option<OpenPick>,
    resolved: Vec<ResolvedPick>,
}

impl ClientShellState {
    /// Shows a picker from the presented endpoint. Returns whether to repaint.
    pub(crate) fn receive_pick_open(
        &mut self,
        origin: ControlOrigin,
        open: EndpointPickOpen,
    ) -> bool {
        if origin.endpoint_id != self.active_endpoint_id {
            self.picks.resolved.push(ResolvedPick {
                origin,
                pick_id: open.pick_id,
                outcome: UiPickOutcome::Cancelled,
            });
            return false;
        }
        self.cancel_pick();
        let pick = ClientPickOverlay {
            pick_id: open.pick_id,
            title: open.title,
            items: open.items,
            create: open.create,
            query: TextEditor::default(),
            selected: 0,
        };
        let selected = open.selected.as_deref().and_then(|id| {
            pick.rows().iter().position(
                |row| matches!(row, ClientPickRow::Item(index) if pick.items[*index].id == id),
            )
        });
        self.picks.open = Some(OpenPick {
            origin,
            pick_id: pick.pick_id.clone(),
        });
        self.overlay = Some(ClientShellOverlay::Pick(ClientPickOverlay {
            selected: selected.unwrap_or(0),
            ..pick
        }));
        true
    }

    /// Closes a picker the daemon resolved without this client, which
    /// therefore does not answer it. Returns whether to repaint.
    pub(crate) fn receive_pick_close(
        &mut self,
        origin: &ControlOrigin,
        close: EndpointPickClose,
    ) -> bool {
        if self
            .picks
            .open
            .take_if(|open| &open.origin == origin && open.pick_id == close.pick_id)
            .is_none()
        {
            return false;
        }
        let showing = matches!(
            &self.overlay,
            Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == close.pick_id
        );
        if showing {
            self.overlay = None;
        }
        showing
    }

    /// Dismisses the open picker, if any, and answers it `cancelled`.
    pub(super) fn cancel_pick(&mut self) {
        let Some(open) = self.picks.open.take() else {
            return;
        };
        if matches!(
            &self.overlay,
            Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == open.pick_id
        ) {
            self.overlay = None;
        }
        self.picks.resolved.push(ResolvedPick {
            origin: open.origin,
            pick_id: open.pick_id,
            outcome: UiPickOutcome::Cancelled,
        });
    }

    /// Returns every picker answer that is ready, after answering `cancelled`
    /// for an open picker whose overlay something else removed. Called once
    /// per client-loop turn.
    pub(crate) fn settle_picks(&mut self) -> Vec<ResolvedPick> {
        let showing = self.picks.open.as_ref().is_some_and(|open| {
            matches!(
                &self.overlay,
                Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == open.pick_id
            )
        });
        if !showing {
            self.cancel_pick();
        }
        std::mem::take(&mut self.picks.resolved)
    }

    pub(super) fn insert_pick_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::Pick(pick)) = self.overlay.as_mut() else {
            return false;
        };
        if pick.query.insert(text) {
            pick.selected = 0;
        }
        true
    }

    pub(super) fn route_pick_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(ClientShellOverlay::Pick(pick)) = self.overlay.as_mut() else {
            return false;
        };
        let (code, modifiers) = crate::config::normalize_key_combo((key.code, key.modifiers));
        outcome.repaint = true;
        match code {
            KeyCode::Esc => self.resolve_pick(Some(UiPickOutcome::Cancelled), outcome),
            KeyCode::Enter => {
                let picked = pick.outcome();
                if picked.is_some() {
                    self.resolve_pick(picked, outcome);
                }
            }
            KeyCode::Up => self.move_pick_selection(-1),
            KeyCode::Down => self.move_pick_selection(1),
            KeyCode::Char('n' | 'p') if modifiers == KeyModifiers::CONTROL => {
                self.move_pick_selection(if code == KeyCode::Char('n') { 1 } else { -1 })
            }
            _ => {
                if pick.query.handle_key(key) == Some(true) {
                    pick.selected = 0;
                }
            }
        }
        true
    }

    pub(super) fn move_pick_selection(&mut self, delta: isize) {
        let Some(ClientShellOverlay::Pick(pick)) = self.overlay.as_mut() else {
            return;
        };
        let last = pick.rows().len().saturating_sub(1) as isize;
        pick.selected = (pick.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Answers the open picker with the selected row, or cancels it.
    pub(super) fn resolve_pick(
        &mut self,
        outcome_choice: Option<UiPickOutcome>,
        outcome: &mut ClientShellInput,
    ) {
        let Some(ClientShellOverlay::Pick(pick)) = self.overlay.take() else {
            return;
        };
        outcome.repaint = true;
        let Some(open) = self.picks.open.take_if(|open| open.pick_id == pick.pick_id) else {
            return;
        };
        let choice = outcome_choice
            .or_else(|| pick.outcome())
            .unwrap_or(UiPickOutcome::Cancelled);
        // The picker has closed, so its plugin must hear something. Text too
        // long for one command could never be sent: the picker is cancelled
        // instead, as Herdr GPUI does, and the user is told why.
        let choice = if crate::client::client_contract::pick_answer_fits(&open.pick_id, &choice) {
            choice
        } else {
            self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                "ui.pick.resolve:too_large",
                "Picker cancelled",
                "The text for the new entry is too long to send.",
            );
            UiPickOutcome::Cancelled
        };
        self.picks.resolved.push(ResolvedPick {
            origin: open.origin,
            pick_id: open.pick_id,
            outcome: choice,
        });
    }
}
