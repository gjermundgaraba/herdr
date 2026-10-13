//! `agent.mark_unread`: hold an acknowledged completion so that its badge
//! comes back once the pane is no longer presented, and stays until the pane
//! is presented again.
//!
//! The hold is daemon state, so every client shows the same status. This
//! module owns the transitions; the server decides when a pane is presented,
//! because that depends on its attached clients
//! (`server/headless/unread_holds.rs`). The hold a pane carries is pane
//! data (`pane/unread_hold.rs`).

use crate::detect::AgentState;
use crate::layout::PaneId;
use crate::pane::unread_hold::UnreadHold;

use super::api::responses::{encode_error, encode_success};
use super::{App, AppState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkUnreadError {
    /// The pane is gone or hosts no agent.
    NotAgent,
    /// Only an acknowledged completion, shown as idle, can be held.
    NotIdle,
}

impl AppState {
    /// Allocates the next agent ordering event: its lifecycle sequence and
    /// its stamp on the priority clock.
    pub(crate) fn next_agent_order_event(&mut self) -> (u64, u64) {
        self.next_agent_state_change_seq += 1;
        self.last_agent_order_stamp =
            crate::agent_priority::next_stamp(self.last_agent_order_stamp);
        (
            self.next_agent_state_change_seq,
            self.last_agent_order_stamp,
        )
    }

    /// Holds the pane's acknowledged completion until `client_id` stops
    /// presenting the pane. Marking an armed pane again moves the hold to the
    /// new client.
    pub(crate) fn arm_unread_hold(
        &mut self,
        ws_idx: usize,
        pane_id: PaneId,
        client_id: Option<u64>,
    ) -> Result<(), MarkUnreadError> {
        let pane = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|workspace| workspace.pane_state_mut(pane_id))
            .ok_or(MarkUnreadError::NotAgent)?;
        let terminal = self
            .terminals
            .get(&pane.attached_terminal_id)
            .filter(|terminal| terminal.is_agent_terminal())
            .ok_or(MarkUnreadError::NotAgent)?;
        if terminal.state != AgentState::Idle || !pane.seen {
            return Err(MarkUnreadError::NotIdle);
        }
        pane.unread_hold = Some(UnreadHold::Armed { client_id });
        self.unread_holds_pending = true;
        Ok(())
    }

    /// Brings an armed completion back: the agent shows as done again and
    /// queues behind the completions already waiting, as a new ordering
    /// event.
    pub(crate) fn restore_unread_hold(&mut self, ws_idx: usize, pane_id: PaneId) -> bool {
        let Some(pane) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|workspace| workspace.pane_state_mut(pane_id))
            .filter(|pane| matches!(pane.unread_hold, Some(UnreadHold::Armed { .. })))
        else {
            return false;
        };
        pane.unread_hold = Some(UnreadHold::Restored);
        pane.seen = false;
        let terminal_id = pane.attached_terminal_id.clone();
        let (sequence, stamp) = self.next_agent_order_event();
        if let Some(terminal) = self.terminals.get_mut(&terminal_id) {
            terminal.last_agent_state_change_seq = Some(sequence);
            terminal.last_agent_completion_seq = Some(sequence);
            terminal.agent_order_stamp = Some(stamp);
        }
        true
    }

    /// Ends a restored hold because the pane is presented again.
    pub(crate) fn clear_unread_hold(&mut self, ws_idx: usize, pane_id: PaneId) -> bool {
        let Some(pane) = self
            .workspaces
            .get_mut(ws_idx)
            .and_then(|workspace| workspace.pane_state_mut(pane_id))
            .filter(|pane| pane.unread_hold == Some(UnreadHold::Restored))
        else {
            return false;
        };
        pane.unread_hold = None;
        pane.seen = true;
        true
    }

    /// Every pane with a hold, by workspace index.
    pub(crate) fn unread_holds(&self) -> Vec<(usize, PaneId, UnreadHold)> {
        self.workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, workspace)| {
                workspace.tabs.iter().flat_map(move |tab| {
                    tab.panes.iter().filter_map(move |(&pane_id, pane)| {
                        pane.unread_hold.map(|hold| (ws_idx, pane_id, hold))
                    })
                })
            })
            .collect()
    }
}

impl App {
    /// `agent.mark_unread`: the calling client holds the pane's completion,
    /// or nobody does when the request came through the public API.
    pub(super) fn handle_agent_mark_unread(
        &mut self,
        id: String,
        target: crate::api::schema::PaneTarget,
    ) -> String {
        let Some((ws_idx, pane_id)) = self.parse_pane_id(&target.pane_id) else {
            return encode_error(
                id,
                "pane_not_found",
                format!("pane {} not found", target.pane_id),
            );
        };
        match self
            .state
            .arm_unread_hold(ws_idx, pane_id, self.invoking_client_id)
        {
            Ok(()) => {}
            Err(MarkUnreadError::NotAgent) => {
                return encode_error(
                    id,
                    "agent_not_found",
                    format!("pane {} hosts no agent", target.pane_id),
                );
            }
            Err(MarkUnreadError::NotIdle) => {
                return encode_error(
                    id,
                    "agent_not_idle",
                    "only an acknowledged completion, shown as idle, can be marked unread",
                );
            }
        }
        match self.agent_info(ws_idx, pane_id) {
            Some(agent) => {
                encode_success(id, crate::api::schema::ResponseResult::AgentInfo { agent })
            }
            None => encode_error(id, "agent_not_found", "agent is no longer available"),
        }
    }

    /// Restores an armed hold and tells subscribers the agent is done again.
    /// The event skips plugin hooks: the user asked for the badge, so nothing
    /// finished that a hook should announce a second time.
    pub(crate) fn restore_unread_hold(&mut self, ws_idx: usize, pane_id: PaneId) -> bool {
        if !self.state.restore_unread_hold(ws_idx, pane_id) {
            return false;
        }
        if let Some(pane) = self.pane_info(ws_idx, pane_id) {
            self.event_hub.push(crate::api::schema::EventEnvelope {
                event: crate::api::schema::EventKind::PaneAgentStatusChanged,
                data: crate::api::schema::EventData::PaneAgentStatusChanged {
                    pane_id: pane.pane_id,
                    workspace_id: pane.workspace_id,
                    agent_status: pane.agent_status,
                    agent: pane.agent,
                    title: pane.title,
                    display_agent: pane.display_agent,
                    state_labels: pane.state_labels,
                },
            });
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;
    use crate::app::api_helpers::pane_agent_status;
    use crate::detect::Agent;
    use crate::workspace::Workspace;

    /// One agent per workspace, each idle and seen.
    fn state(count: usize) -> AppState {
        let mut state = AppState::test_new();
        state.workspaces = (0..count)
            .map(|index| Workspace::test_new(&format!("space-{index}")))
            .collect();
        state.ensure_test_terminals();
        state.active = Some(0);
        for ws_idx in 0..count {
            let terminal_id = pane(&state, ws_idx).attached_terminal_id.clone();
            let terminal = state.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(Agent::Claude);
            terminal.state = AgentState::Idle;
        }
        state
    }

    fn pane_id(state: &AppState, ws_idx: usize) -> PaneId {
        state.workspaces[ws_idx].tabs[0].root_pane
    }

    fn pane(state: &AppState, ws_idx: usize) -> &crate::pane::PaneState {
        let pane_id = pane_id(state, ws_idx);
        state.workspaces[ws_idx].pane_state(pane_id).unwrap()
    }

    fn terminal(state: &AppState, ws_idx: usize) -> &crate::terminal::TerminalState {
        &state.terminals[&pane(state, ws_idx).attached_terminal_id]
    }

    fn status(state: &AppState, ws_idx: usize) -> AgentStatus {
        pane_agent_status(terminal(state, ws_idx).state, pane(state, ws_idx).seen)
    }

    fn set_seen(state: &mut AppState, ws_idx: usize, seen: bool) {
        let pane_id = pane_id(state, ws_idx);
        state.workspaces[ws_idx]
            .pane_state_mut(pane_id)
            .unwrap()
            .seen = seen;
    }

    fn complete(state: &mut AppState, ws_idx: usize) {
        let terminal_id = pane(state, ws_idx).attached_terminal_id.clone();
        let (sequence, stamp) = state.next_agent_order_event();
        let terminal = state.terminals.get_mut(&terminal_id).unwrap();
        terminal.last_agent_state_change_seq = Some(sequence);
        terminal.last_agent_completion_seq = Some(sequence);
        terminal.agent_order_stamp = Some(stamp);
        set_seen(state, ws_idx, false);
    }

    #[test]
    fn only_an_acknowledged_completion_can_be_held() {
        let mut state = state(1);
        let target = pane_id(&state, 0);

        let terminal_id = pane(&state, 0).attached_terminal_id.clone();
        for busy in [
            AgentState::Working,
            AgentState::Blocked,
            AgentState::Unknown,
        ] {
            state.terminals.get_mut(&terminal_id).unwrap().state = busy;
            assert_eq!(
                state.arm_unread_hold(0, target, Some(7)),
                Err(MarkUnreadError::NotIdle)
            );
        }
        state.terminals.get_mut(&terminal_id).unwrap().state = AgentState::Idle;

        // A completion nobody has seen yet still shows its badge.
        set_seen(&mut state, 0, false);
        assert_eq!(
            state.arm_unread_hold(0, target, Some(7)),
            Err(MarkUnreadError::NotIdle)
        );
        set_seen(&mut state, 0, true);

        state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .detected_agent = None;
        assert_eq!(
            state.arm_unread_hold(0, target, Some(7)),
            Err(MarkUnreadError::NotAgent)
        );
        assert_eq!(
            state.arm_unread_hold(9, target, Some(7)),
            Err(MarkUnreadError::NotAgent)
        );
        assert!(state.unread_holds().is_empty());
    }

    #[test]
    fn a_hold_stays_idle_until_it_is_restored_as_a_new_completion() {
        let mut state = state(1);
        let target = pane_id(&state, 0);
        complete(&mut state, 0);
        state.mark_active_tab_seen();
        let acknowledged = terminal(&state, 0).last_agent_state_change_seq.unwrap();

        assert_eq!(state.arm_unread_hold(0, target, Some(7)), Ok(()));
        assert_eq!(status(&state, 0), AgentStatus::Idle);
        assert_eq!(
            state.unread_holds(),
            [(0, target, UnreadHold::Armed { client_id: Some(7) })]
        );
        // Marking again moves the hold to the newer client.
        assert_eq!(state.arm_unread_hold(0, target, Some(8)), Ok(()));
        assert_eq!(
            pane(&state, 0).unread_hold,
            Some(UnreadHold::Armed { client_id: Some(8) })
        );
        assert!(!state.clear_unread_hold(0, target));

        assert!(state.restore_unread_hold(0, target));
        assert_eq!(status(&state, 0), AgentStatus::Done);
        let restored = terminal(&state, 0);
        let sequence = restored.last_agent_state_change_seq.unwrap();
        assert!(sequence > acknowledged);
        assert_eq!(restored.last_agent_completion_seq, Some(sequence));
        assert!(!state.restore_unread_hold(0, target));
        assert_eq!(
            state.arm_unread_hold(0, target, Some(7)),
            Err(MarkUnreadError::NotIdle)
        );
    }

    #[test]
    fn a_restored_hold_survives_tab_wide_seen_marks_until_cleared() {
        let mut state = state(1);
        let target = pane_id(&state, 0);
        state.arm_unread_hold(0, target, None).unwrap();
        state.restore_unread_hold(0, target);

        assert!(!state.mark_active_tab_seen());
        state.workspaces[0].switch_tab(0);
        assert_eq!(status(&state, 0), AgentStatus::Done);

        assert!(state.clear_unread_hold(0, target));
        assert_eq!(status(&state, 0), AgentStatus::Idle);
        assert!(state.unread_holds().is_empty());
        assert!(!state.clear_unread_hold(0, target));
    }

    #[test]
    fn a_restored_completion_queues_behind_the_ones_already_waiting() {
        let mut state = state(3);
        let key = |state: &AppState, ws_idx: usize| {
            crate::agent_priority::priority_key(
                false,
                status(state, ws_idx),
                terminal(state, ws_idx).agent_order_stamp.unwrap_or(0),
            )
        };
        complete(&mut state, 0);
        state.mark_active_tab_seen();
        complete(&mut state, 1);
        complete(&mut state, 2);
        let target = pane_id(&state, 0);
        state.arm_unread_hold(0, target, None).unwrap();
        // While armed the agent is idle, behind every waiting agent.
        assert!(key(&state, 1) < key(&state, 2));
        assert!(key(&state, 2) < key(&state, 0));

        state.restore_unread_hold(0, target);

        // Restored, it is the youngest completion in the done group.
        assert!(key(&state, 1) < key(&state, 2));
        assert!(key(&state, 2) < key(&state, 0));
        assert!(key(&state, 0).starts_with("1.1."));

        // A later completion queues behind it.
        set_seen(&mut state, 1, true);
        complete(&mut state, 1);
        assert!(key(&state, 0) < key(&state, 1));

        // Seen again, it is the freshest idle agent.
        state.clear_unread_hold(0, target);
        set_seen(&mut state, 2, true);
        assert!(key(&state, 0) < key(&state, 2));
        assert!(key(&state, 0).starts_with("1.3."));
    }
}
