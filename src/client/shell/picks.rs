//! `ui.pick` pickers the server asks this client to show. Every exit path
//! answers the server with `ui.pick.resolve`, so the waiting plugin never
//! hangs on a picker the user dismissed.

use crossterm::event::{KeyCode, KeyModifiers};

use super::*;
use crate::api::schema::{Method, UiPickOutcome, UiPickResolveParams};
use crate::protocol::endpoint::{EndpointPickClose, EndpointPickOpen};

impl ClientShellState {
    /// Shows a picker from the active endpoint. Returns whether to repaint.
    pub(crate) fn receive_pick_open(
        &mut self,
        endpoint_id: &ClientEndpointId,
        open: EndpointPickOpen,
    ) -> bool {
        if endpoint_id != &self.active_endpoint_id {
            return false;
        }
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
        self.overlay = Some(ClientShellOverlay::Pick(ClientPickOverlay {
            selected: selected.unwrap_or(0),
            ..pick
        }));
        true
    }

    /// Closes a picker the server resolved without this client.
    pub(crate) fn receive_pick_close(
        &mut self,
        endpoint_id: &ClientEndpointId,
        close: EndpointPickClose,
    ) -> bool {
        let showing = endpoint_id == &self.active_endpoint_id
            && matches!(
                &self.overlay,
                Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == close.pick_id
            );
        if showing {
            self.overlay = None;
        }
        showing
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
        let choice = outcome_choice
            .or_else(|| pick.outcome())
            .unwrap_or(UiPickOutcome::Cancelled);
        self.push_endpoint_method(
            Method::UiPickResolve(UiPickResolveParams {
                pick_id: pick.pick_id,
                outcome: choice,
            }),
            outcome,
        );
        outcome.repaint = true;
    }
}
