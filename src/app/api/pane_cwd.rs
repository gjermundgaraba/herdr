//! `pane.cwd_changed`: emitted when a shell's cwd report changes its pane's cwd.
//! Also the public tab id that it, `pane.closed` and `pane.exited` carry.

use crate::app::App;
use crate::events::AppEvent;
use crate::layout::PaneId;

impl App {
    pub(super) fn public_tab_id_for_pane(&self, ws_idx: usize, pane_id: PaneId) -> Option<String> {
        let tab_idx = self
            .state
            .workspaces
            .get(ws_idx)?
            .find_tab_index_for_pane(pane_id)?;
        self.public_tab_id(ws_idx, tab_idx)
    }

    /// The reporting pane and its cwd before a cwd report is applied.
    pub(super) fn cwd_before_report(
        &self,
        event: &AppEvent,
    ) -> Option<(PaneId, Option<std::path::PathBuf>)> {
        let AppEvent::TerminalCwdReported { pane_id, .. } = event else {
            return None;
        };
        Some((*pane_id, self.pane_terminal_cwd(*pane_id)))
    }

    fn pane_terminal_cwd(&self, pane_id: PaneId) -> Option<std::path::PathBuf> {
        let (ws_idx, _) = self.find_pane(pane_id)?;
        let terminal_id = self.state.terminal_id_for_pane(ws_idx, pane_id)?;
        Some(self.state.terminals.get(&terminal_id)?.cwd.clone())
    }

    /// Emits `pane.cwd_changed` when the report changed the pane's cwd.
    pub(super) fn emit_pane_cwd_changed(
        &mut self,
        reported: Option<(PaneId, Option<std::path::PathBuf>)>,
    ) {
        let Some((pane_id, before)) = reported else {
            return;
        };
        let Some(cwd) = self
            .pane_terminal_cwd(pane_id)
            .filter(|cwd| Some(cwd) != before.as_ref())
        else {
            return;
        };
        let Some((ws_idx, _)) = self.find_pane(pane_id) else {
            return;
        };
        let (Some(public_pane_id), Some(tab_id)) = (
            self.public_pane_id(ws_idx, pane_id),
            self.public_tab_id_for_pane(ws_idx, pane_id),
        ) else {
            return;
        };
        self.emit_event(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::PaneCwdChanged,
            data: crate::api::schema::EventData::PaneCwdChanged {
                pane_id: public_pane_id,
                workspace_id: self.public_workspace_id(ws_idx),
                tab_id,
                cwd: cwd.display().to_string(),
            },
        });
    }
}
