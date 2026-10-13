//! Agent state a program reports in its own output (OSC 7501, read by
//! `pane/program_status.rs`), applied through the same path as socket hook
//! reports but without their sequence numbers.

use std::time::Instant;

use super::{TerminalState, TerminalStateMutation};
use crate::detect::AgentState;

/// How a hook report is ordered against earlier reports from the same source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HookReportOrdering {
    /// Socket reports carry a sequence number; older numbers are dropped.
    Sequenced(Option<u64>),
    /// Reports read from the agent's own output arrive in order already.
    InBand,
}

impl TerminalState {
    /// Applies a state a program reported in its own output (OSC 7501) to the session a
    /// full-lifecycle integration reported for the agent named `app`. The output stream
    /// already orders these reports, so they leave the socket's sequence numbers alone. They
    /// carry no session, so they keep the reported one and wait until there is one.
    pub fn set_in_band_hook_state_at(
        &mut self,
        app: &str,
        state: AgentState,
        now: Instant,
    ) -> Option<TerminalStateMutation> {
        let (source, agent_label, session_ref) = self.in_band_anchor(app)?;
        self.apply_hook_report_at(
            source,
            agent_label,
            state,
            None,
            Some(session_ref),
            HookReportOrdering::InBand,
            now,
        )
    }

    /// The full-lifecycle source, label and session an integration reported for `app`.
    fn in_band_anchor(
        &self,
        app: &str,
    ) -> Option<(String, String, crate::agent_resume::AgentSessionRef)> {
        let authority = self.hook_authority.as_ref().and_then(|authority| {
            Some((
                &authority.source,
                &authority.agent_label,
                authority.session_ref.as_ref()?,
            ))
        });
        let persisted = self
            .persisted_agent_session
            .as_ref()
            .map(|session| (&session.source, &session.agent, &session.session_ref));
        authority
            .into_iter()
            .chain(persisted)
            .find(|(source, agent_label, _)| {
                agent_label.as_str() == app
                    && crate::detect::full_lifecycle_hook_authority(source, agent_label)
            })
            .map(|(source, agent_label, session_ref)| {
                (source.clone(), agent_label.clone(), session_ref.clone())
            })
    }
}
