//! Back and Forward through the panes an endpoint has focused, the way a
//! browser walks its pages. Each endpoint keeps its own trail, observed from
//! the daemon's snapshots, so focus changed from anywhere (a key, the sidebar,
//! another client) becomes a step, and pane IDs never outlive their boot. The
//! `history_back` and `history_forward` keys step the presented endpoint's
//! trail and never cross to another endpoint.
//!
//! Mirrored client logic: a port of Herdr GPUI's
//! `crates/herdr-gpui/src/endpoint/history.rs`, logic and test cases (see
//! "Sibling fork: Herdr GPUI" in `FORK.md`).

use std::collections::{HashMap, VecDeque};
use std::sync::LazyLock;

use super::*;
use crate::input::KeybindAction;

/// How many panes the trail remembers; older ones fall off the back.
const LIMIT: usize = 100;

/// The method name a pending pane focus request carries.
static PANE_FOCUS: LazyLock<&str> = LazyLock::new(|| {
    crate::api::api_method_name(&crate::api::schema::Method::PaneFocus(
        crate::api::schema::PaneTarget {
            pane_id: String::new(),
        },
    ))
});

/// Which way along the trail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Back,
    Forward,
}

#[derive(Debug, Default)]
pub(super) struct History {
    boot: String,
    panes: VecDeque<String>,
    /// The entry the focused pane was reached as.
    cursor: usize,
    /// The entry a Back or Forward asked the daemon for. Until its focus
    /// arrives, the next step continues from there, and the arriving focus
    /// moves the cursor instead of cutting off the entries ahead.
    travel: Option<usize>,
}

/// Each endpoint's trail. The entry of an endpoint that went away is kept: a
/// snapshot of another boot resets it, so it never steps to a stale pane.
pub(super) type Trails = HashMap<ClientEndpointId, History>;

impl History {
    /// Records the snapshot's focus. A snapshot without one, such as the
    /// empty state between connections, leaves the trail alone.
    ///
    /// `navigating` says whether a focus request is still in flight. A
    /// travel lives only that long: once the daemon has answered, failed, or
    /// the connection carrying it is gone, whatever focus arrived is the
    /// outcome, and later steps start from where the user really is.
    pub(super) fn observe(&mut self, snapshot: Option<&ClientShellSnapshot>, navigating: bool) {
        if let Some(snapshot) = snapshot {
            self.record(snapshot);
        }
        if !navigating {
            self.travel = None;
        }
    }

    fn record(&mut self, snapshot: &ClientShellSnapshot) {
        if snapshot.boot_id != self.boot {
            *self = Self {
                boot: snapshot.boot_id.clone(),
                ..Self::default()
            };
        }
        let Some(focused) = snapshot.focused_pane_id.as_deref() else {
            return;
        };
        if self.entry(self.cursor) == Some(focused) {
            return;
        }
        if let Some(index) = self.travel.take() {
            if self.entry(index) == Some(focused) {
                self.cursor = index;
                return;
            }
        }
        // Somewhere new: like a browser, it replaces whatever was ahead.
        self.panes.truncate(self.cursor + 1);
        self.panes.push_back(focused.to_owned());
        if self.panes.len() > LIMIT {
            self.panes.pop_front();
        }
        self.cursor = self.panes.len() - 1;
    }

    /// Whether `step` has a pane to go to in `snapshot`.
    #[cfg(test)]
    pub(super) fn can(&self, step: Step, snapshot: &ClientShellSnapshot) -> bool {
        self.target(step, snapshot).is_some()
    }

    /// The entry `step` would go to and its pane, without going. The caller
    /// asks the daemon to focus the pane and calls [`Self::begin`] only once
    /// the request is queued, so a press that cannot be sent changes nothing.
    pub(super) fn peek(&self, step: Step, snapshot: &ClientShellSnapshot) -> Option<(usize, &str)> {
        let index = self.target(step, snapshot)?;
        Some((index, self.entry(index)?))
    }

    /// Marks `index` as the travel in flight.
    pub(super) fn begin(&mut self, index: usize) {
        if index < self.panes.len() {
            self.travel = Some(index);
        }
    }

    /// The nearest entry in `step`'s direction whose pane still exists and is
    /// not the one already focused. Closed panes stay in the trail and are
    /// stepped over, as Herdr's `last_pane` checks its pane.
    fn target(&self, step: Step, snapshot: &ClientShellSnapshot) -> Option<usize> {
        if snapshot.boot_id != self.boot {
            return None;
        }
        let from = self.travel.unwrap_or(self.cursor);
        let usable = |&index: &usize| {
            self.entry(index).is_some_and(|pane| {
                snapshot.focused_pane_id.as_deref() != Some(pane)
                    && snapshot.panes.iter().any(|p| p.pane_id == pane)
            })
        };
        match step {
            Step::Back => (0..from).rev().find(usable),
            Step::Forward => (from + 1..self.panes.len()).find(usable),
        }
    }

    fn entry(&self, index: usize) -> Option<&str> {
        self.panes.get(index).map(String::as_str)
    }
}

impl ClientShellState {
    /// Records an endpoint's snapshot on its trail, whether it is presented
    /// or not. Focus requests go only to the presented endpoint, so only its
    /// travel can still be in flight.
    pub(super) fn observe_history(
        &mut self,
        endpoint_id: &ClientEndpointId,
        snapshot: &ClientShellSnapshot,
    ) {
        let navigating = endpoint_id == &self.active_endpoint_id && self.focus_request_in_flight();
        self.fork
            .history
            .entry(endpoint_id.clone())
            .or_default()
            .observe(Some(snapshot), navigating);
    }

    /// Whether a pane focus request awaits its answer, which is how long a
    /// travel lasts. Requests go only to the presented endpoint.
    fn focus_request_in_flight(&self) -> bool {
        self.pending_requests
            .values()
            .any(|request| request.method_name == *PANE_FOCUS)
    }

    /// Runs `history_back` and `history_forward`; false for any other action.
    pub(super) fn history_key(
        &mut self,
        action: KeybindAction,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let step = match action {
            KeybindAction::HistoryBack => Step::Back,
            KeybindAction::HistoryForward => Step::Forward,
            _ => return false,
        };
        self.travel(step, outcome);
        true
    }

    /// Back or Forward along the presented endpoint's trail. The trail
    /// remembers the step only once its focus request is queued, so a press
    /// that cannot be sent changes nothing.
    ///
    /// A travel whose request is gone, answered, refused, or cancelled
    /// before it was sent, ends here rather than at the next snapshot, so
    /// the step starts from where the user is.
    fn travel(&mut self, step: Step, outcome: &mut ClientShellInput) {
        let navigating = self.focus_request_in_flight();
        if let Some(history) = self.fork.history.get_mut(&self.active_endpoint_id) {
            history.observe(None, navigating);
        }
        let Some((index, pane_id)) = self.snapshot.as_deref().and_then(|snapshot| {
            let (index, pane_id) = self
                .fork
                .history
                .get(&self.active_endpoint_id)?
                .peek(step, snapshot)?;
            Some((index, pane_id.to_owned()))
        }) else {
            return;
        };
        let method =
            crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget { pane_id });
        if self.push_endpoint_method_with_kind(method, PendingEndpointKind::Generic, outcome) {
            if let Some(history) = self.fork.history.get_mut(&self.active_endpoint_id) {
                history.begin(index);
            }
        }
    }
}

#[cfg(test)]
mod tests;
