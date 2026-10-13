//! Agent state from the Program Status Protocol (OSC 7501): the state reports a
//! program sends about itself in its own output. `osc.rs` hands each OSC 7501
//! body to [`parse_program_status_report`], and the pane runtime publishes the
//! reports as [`AppEvent::InBandHookStateReported`].
//!
//! Herdr does not answer the protocol's support query. Pi reports without
//! asking only when it runs with `PI_PROGRAM_STATUS=1`, which the user sets in
//! the shell when `HERDR_ENV` is set; without it Pi asks, gets no answer, and
//! never reports.

use tokio::sync::mpsc;
use tracing::warn;

use crate::detect::AgentState;
use crate::events::AppEvent;
use crate::layout::PaneId;

/// An OSC 7501 state report from the program named by `app`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgramStatusReport {
    app: String,
    state: AgentState,
}

/// Reads `state=<state>:app=<name>[:...]`, ignoring other keys. Herdr has no done or
/// error state: both end a run like idle, which Herdr shows as done until seen.
/// `clear` only means the program stopped drawing, so it reports nothing, and
/// neither does a support query (`?`).
pub(super) fn parse_program_status_report(payload: &[u8]) -> Option<ProgramStatusReport> {
    let mut state = None;
    let mut app = None;
    for pair in payload.split(|&byte| byte == b':') {
        let Some(separator) = pair.iter().position(|&byte| byte == b'=') else {
            continue;
        };
        match (&pair[..separator], &pair[separator + 1..]) {
            (b"state", value) => {
                state = match value {
                    b"working" => Some(AgentState::Working),
                    b"blocked" => Some(AgentState::Blocked),
                    b"idle" | b"done" | b"error" => Some(AgentState::Idle),
                    _ => return None,
                };
            }
            (b"app", value) => app = std::str::from_utf8(value).ok(),
            _ => {}
        }
    }
    Some(ProgramStatusReport {
        app: app?.to_string(),
        state: state?,
    })
}

pub(super) fn publish(
    pane_id: PaneId,
    reports: Vec<ProgramStatusReport>,
    events: &mpsc::Sender<AppEvent>,
) {
    for ProgramStatusReport { app, state } in reports {
        if let Err(err) = events.try_send(AppEvent::InBandHookStateReported {
            pane_id,
            app,
            state,
        }) {
            warn!(
                pane = pane_id.raw(),
                err = %err,
                "failed to queue program status report"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::osc::AgentOscStateTracker;
    use super::super::terminal::GhosttyPaneTerminal;
    use super::*;

    fn report(app: &str, state: AgentState) -> ProgramStatusReport {
        ProgramStatusReport {
            app: app.into(),
            state,
        }
    }

    #[test]
    fn agent_osc_tracker_reads_program_status_reports() {
        let mut tracker = AgentOscStateTracker::default();

        tracker.observe(b"\x1b]7501;state=working:app=pi:msg=V29yaw==\x1b\\");
        tracker.observe(b"\x1b]7501;state=blocked:app=pi:kind=question\x07");
        tracker.observe(b"\x1b]7501;state=done:app=pi\x07\x1b]7501;state=error:app=pi\x07");
        // Clearing, unknown states, reports without an app and support queries say
        // nothing about a run.
        tracker.observe(b"\x1b]7501;state=clear:app=pi\x07\x1b]7501;state=sleeping:app=pi\x07");
        tracker.observe(b"\x1b]7501;state=working\x07\x1b]7501;?\x1b\\");

        assert_eq!(
            tracker.drain_program_status_reports(),
            vec![
                report("pi", AgentState::Working),
                report("pi", AgentState::Blocked),
                report("pi", AgentState::Idle),
                report("pi", AgentState::Idle),
            ]
        );
    }

    #[test]
    fn process_pty_bytes_returns_program_status_reports() {
        let (tx, _rx) = mpsc::channel(4);
        let terminal = crate::ghostty::Terminal::new(20, 5, 0).unwrap();
        let pane = GhosttyPaneTerminal::new(terminal, tx.clone()).unwrap();
        let pane_id = PaneId::from_raw(1);

        let result = pane.process_pty_bytes(pane_id, 0, b"\x1b]7501;state=working:app=pi\x07", &tx);
        assert!(result.terminal_responses.is_empty());
        assert_eq!(
            result.program_status_reports,
            vec![report("pi", AgentState::Working)]
        );
    }
}
