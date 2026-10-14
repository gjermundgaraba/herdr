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

/// The connection a daemon's picker arrived on. Its answer goes back on that
/// connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ControlOrigin {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) generation: u64,
}

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

/// A `ui.pick` picker the server asked this client to show.
#[derive(Debug)]
pub(super) struct ClientPickOverlay {
    pub(super) pick_id: String,
    pub(super) title: String,
    pub(super) items: Vec<crate::api::schema::UiPickItem>,
    pub(super) create: Option<crate::api::schema::UiPickCreate>,
    pub(super) query: TextEditor,
    /// Index into [`ClientPickOverlay::rows`].
    pub(super) selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClientPickRow {
    Item(usize),
    Create,
}

impl ClientPickOverlay {
    /// Items matching the query, then a create row when the query names no item.
    pub(super) fn rows(&self) -> Vec<ClientPickRow> {
        let query = self.query.trim().to_lowercase();
        let mut rows = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty()
                    || format!(
                        "{} {}",
                        item.label,
                        item.detail.as_deref().unwrap_or_default()
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(index, _)| ClientPickRow::Item(index))
            .collect::<Vec<_>>();
        if self.create.is_some()
            && !query.is_empty()
            && !self
                .items
                .iter()
                .any(|item| item.label.trim().to_lowercase() == query)
        {
            rows.push(ClientPickRow::Create);
        }
        rows
    }

    pub(super) fn outcome(&self) -> Option<crate::api::schema::UiPickOutcome> {
        Some(match *self.rows().get(self.selected)? {
            ClientPickRow::Item(index) => crate::api::schema::UiPickOutcome::Picked {
                id: self.items[index].id.clone(),
            },
            ClientPickRow::Create => crate::api::schema::UiPickOutcome::Created {
                text: self.query.trim().to_owned(),
            },
        })
    }
}

impl ClientShellState {
    /// Shows a picker from the presented endpoint. Returns whether to repaint.
    pub(crate) fn receive_pick_open(
        &mut self,
        origin: ControlOrigin,
        open: EndpointPickOpen,
    ) -> bool {
        if origin.endpoint_id != self.active_endpoint_id {
            self.fork.picks.resolved.push(ResolvedPick {
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
        self.fork.picks.open = Some(OpenPick {
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
            .fork
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
        let Some(open) = self.fork.picks.open.take() else {
            return;
        };
        if matches!(
            &self.overlay,
            Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == open.pick_id
        ) {
            self.overlay = None;
        }
        self.fork.picks.resolved.push(ResolvedPick {
            origin: open.origin,
            pick_id: open.pick_id,
            outcome: UiPickOutcome::Cancelled,
        });
    }

    /// Returns every picker answer that is ready, after answering `cancelled`
    /// for an open picker whose overlay something else removed. Called once
    /// per client-loop turn.
    pub(crate) fn settle_picks(&mut self) -> Vec<ResolvedPick> {
        let showing = self.fork.picks.open.as_ref().is_some_and(|open| {
            matches!(
                &self.overlay,
                Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == open.pick_id
            )
        });
        if !showing {
            self.cancel_pick();
        }
        std::mem::take(&mut self.fork.picks.resolved)
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

    /// Handles the mouse while a picker is open: the wheel moves the
    /// selection, and a click picks a row or answers the buttons. Returns
    /// whether a picker took the event.
    pub(super) fn pick_mouse(
        &mut self,
        mouse: crossterm::event::MouseEvent,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};
        if !matches!(self.overlay, Some(ClientShellOverlay::Pick(_))) {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_pick_selection(-1);
                outcome.repaint = true;
            }
            MouseEventKind::ScrollDown => {
                self.move_pick_selection(1);
                outcome.repaint = true;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let row = self
                    .hits
                    .worktree_rows
                    .iter()
                    .find(|(rect, _)| super::contains(*rect, point))
                    .map(|(_, index)| *index);
                if super::contains(self.hits.overlay_cancel, point) {
                    self.resolve_pick(Some(crate::api::schema::UiPickOutcome::Cancelled), outcome);
                } else if let Some(index) = row {
                    if let Some(ClientShellOverlay::Pick(pick)) = self.overlay.as_mut() {
                        pick.selected = index;
                    }
                    self.resolve_pick(None, outcome);
                } else if super::contains(self.hits.overlay_primary, point)
                    && matches!(&self.overlay, Some(ClientShellOverlay::Pick(pick)) if pick.outcome().is_some())
                {
                    self.resolve_pick(None, outcome);
                }
            }
            _ => {}
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
        let Some(open) = self
            .fork
            .picks
            .open
            .take_if(|open| open.pick_id == pick.pick_id)
        else {
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
        self.fork.picks.resolved.push(ResolvedPick {
            origin: open.origin,
            pick_id: open.pick_id,
            outcome: choice,
        });
    }
}
