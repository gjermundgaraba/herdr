//! Bounded, coalesced Back/Forward history of pane visits across endpoints.
//!
//! A visit is recorded once per accepted active-endpoint snapshot whose focused
//! pane differs from the current entry, so rapid intermediate focus changes that
//! never produced a snapshot are not visits. Traversal moves the cursor
//! optimistically; the observation that lands on the traversal target is then
//! recognised instead of being pushed as a new visit, and a failed traversal
//! returns the cursor to where it started. Visits to panes an endpoint no longer
//! reports are pruned whenever that endpoint's snapshot is accepted.
//!
//! While a traversal's navigation is unanswered, nothing observed is a visit
//! or its arrival, and nothing truncates: it is an older snapshot, or another
//! client, a script, or a closing pane moving the focus. That holds for a
//! pane equal to the target too. A history that toggles between two panes
//! holds equal visits, so the pane a chained step is leaving can pass for
//! where it leads. Whatever the user does next on purpose is a navigation of
//! this client, which the endpoint carries out after the step's own. The
//! traversal ends when its target is presented as its navigation is answered
//! or is observed after that, or when its navigation fails; once the
//! navigation was carried out, a pane off the traversed path is a visit
//! again. An answer belongs to one step, told apart by [`Step`] and not by
//! where the step leads, for the same reason. A step to another endpoint
//! sends no request: the client loop activates that endpoint, and its word
//! that the activation completed is the step's answer.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/history.rs` and `history/sync.rs` (see "Sibling fork:
//! Herdr GPUI" in `FORK.md`).
use std::collections::{HashSet, VecDeque};

use crate::client::endpoint::ClientEndpointId;

const CAPACITY: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visit {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) boot_id: String,
    pub(crate) pane_id: String,
}

/// A Back/Forward step as its caller follows it: where it leads, and which
/// step it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub(crate) visit: Visit,
    id: u64,
}

/// An in-flight traversal whose target is the cursor.
#[derive(Debug)]
struct Traversal {
    /// The [`Step`] that began it. A chained step replaces the traversal, so
    /// an earlier step's answer or failure no longer finds it.
    step: u64,
    /// Index of the visit that was current before the traversal began.
    origin: usize,
    /// Whether the endpoint carried out the step's navigation. Until then
    /// nothing observed belongs to this traversal or follows it.
    answered: bool,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    visits: VecDeque<Visit>,
    /// Index of the current visit; meaningful only while `visits` is non-empty.
    cursor: usize,
    /// Traversal whose arrival must not truncate the forward list.
    pending: Option<Traversal>,
    /// Steps begun so far; the next step's id.
    steps: u64,
}

impl History {
    /// Record the focused pane of the active endpoint after an accepted snapshot.
    pub(crate) fn observe(
        &mut self,
        endpoint_id: &ClientEndpointId,
        boot_id: &str,
        pane_id: Option<&str>,
    ) {
        let Some(pane_id) = pane_id else {
            return;
        };
        let visit = Visit {
            endpoint_id: endpoint_id.clone(),
            boot_id: boot_id.to_owned(),
            pane_id: pane_id.to_owned(),
        };
        if let Some(traversal) = &self.pending {
            // Nothing the user did after the step can show before the step
            // itself was carried out, and a pane equal to its target may be
            // the one a chained step is leaving.
            if !traversal.answered {
                return;
            }
            if self.current() == Some(&visit) {
                self.pending = None;
                return;
            }
            // Snapshots taken before the server applied the traversal still show
            // a pane on the traversed path; they are not new visits.
            let low = traversal.origin.min(self.cursor);
            let high = traversal.origin.max(self.cursor);
            if self.visits.range(low..=high).any(|path| path == &visit) {
                return;
            }
        }
        if self.current() == Some(&visit) {
            return;
        }
        self.pending = None;
        if !self.visits.is_empty() {
            self.visits.truncate(self.cursor + 1);
        }
        self.visits.push_back(visit);
        while self.visits.len() > CAPACITY {
            self.visits.pop_front();
        }
        self.cursor = self.visits.len() - 1;
    }

    /// A server reboot invalidates every visit recorded under the old boot.
    pub(crate) fn observe_boot(&mut self, endpoint_id: &ClientEndpointId, boot_id: &str) {
        self.rebuild(|visit| &visit.endpoint_id != endpoint_id || visit.boot_id == boot_id);
    }

    /// Forget visits to panes that an endpoint's accepted snapshot no longer contains.
    pub(crate) fn observe_panes<'a>(
        &mut self,
        endpoint_id: &ClientEndpointId,
        boot_id: &str,
        pane_ids: impl IntoIterator<Item = &'a str>,
    ) {
        let tracked = |visit: &Visit| &visit.endpoint_id == endpoint_id && visit.boot_id == boot_id;
        if !self.visits.iter().any(tracked) {
            return;
        }
        let live: HashSet<&str> = pane_ids.into_iter().collect();
        self.rebuild(|visit| !tracked(visit) || live.contains(visit.pane_id.as_str()));
    }

    /// The endpoint carried out the navigation of `step`. `shown` says
    /// whether the client presents the step's target as the answer arrives:
    /// the endpoint may send the snapshot that shows it ahead of the answer,
    /// where it confirmed nothing. If the step is still the pending one, it
    /// ends here when its target is shown; otherwise the target's next
    /// observation ends it, and a pane off its path is a visit from here on,
    /// whether or not the target itself is ever observed.
    pub(crate) fn traversal_answered(&mut self, step: &Step, shown: bool) {
        let Some(traversal) = self
            .pending
            .as_mut()
            .filter(|traversal| traversal.step == step.id)
        else {
            return;
        };
        if shown {
            self.pending = None;
        } else {
            traversal.answered = true;
        }
    }

    /// The navigation of `step` failed or was refused: if the step is still
    /// the pending one, return to the visit it started from. Forget its
    /// target when the pane no longer exists.
    pub(crate) fn traversal_failed(&mut self, step: &Step, pane_closed: bool) {
        if let Some(traversal) = self.pending.take_if(|traversal| traversal.step == step.id) {
            self.cursor = traversal.origin;
        }
        if pane_closed {
            self.rebuild(|candidate| candidate != &step.visit);
        }
    }

    pub(crate) fn retire(&mut self, endpoint_id: &ClientEndpointId) {
        self.rebuild(|visit| &visit.endpoint_id != endpoint_id);
    }

    pub(crate) fn clear(&mut self) {
        self.visits.clear();
        self.cursor = 0;
        self.pending = None;
    }

    /// Step to the nearest earlier visit on an online endpoint.
    pub(crate) fn back(&mut self, online: &HashSet<ClientEndpointId>) -> Option<Step> {
        let index = (0..self.cursor)
            .rev()
            .find(|index| online.contains(&self.visits[*index].endpoint_id))?;
        self.traverse(index)
    }

    /// Step to the nearest later visit on an online endpoint.
    pub(crate) fn forward(&mut self, online: &HashSet<ClientEndpointId>) -> Option<Step> {
        let index = (self.cursor + 1..self.visits.len())
            .find(|index| online.contains(&self.visits[*index].endpoint_id))?;
        self.traverse(index)
    }

    fn traverse(&mut self, index: usize) -> Option<Step> {
        let visit = self.visits.get(index)?.clone();
        // Chained steps keep the first origin so the whole path stays recognised.
        let origin = self
            .pending
            .as_ref()
            .map_or(self.cursor, |traversal| traversal.origin);
        let id = self.steps;
        self.steps = self.steps.wrapping_add(1);
        self.cursor = index;
        self.pending = Some(Traversal {
            step: id,
            origin,
            answered: false,
        });
        Some(Step { visit, id })
    }

    fn current(&self) -> Option<&Visit> {
        self.visits.get(self.cursor)
    }

    /// Keep the visits matching `keep`, merge neighbours that removals made
    /// equal, and remap the cursor and any in-flight traversal onto survivors.
    /// A dropped cursor moves to the nearest earlier survivor.
    fn rebuild(&mut self, keep: impl Fn(&Visit) -> bool) {
        if self.visits.iter().all(&keep) {
            return;
        }
        let mut survivors = VecDeque::with_capacity(self.visits.len());
        // Old index -> surviving index: itself, the equal neighbour it merged
        // into, or the nearest earlier survivor when dropped.
        let mut mapped = Vec::with_capacity(self.visits.len());
        let mut dropped = Vec::with_capacity(self.visits.len());
        for visit in self.visits.drain(..) {
            let kept = keep(&visit);
            if kept && survivors.back() != Some(&visit) {
                survivors.push_back(visit);
            }
            mapped.push(survivors.len().checked_sub(1));
            dropped.push(!kept);
        }
        self.visits = survivors;
        let remap = |index: usize| mapped.get(index).copied().flatten().unwrap_or(0);
        let was_dropped = |index: usize| dropped.get(index).copied().unwrap_or(true);
        let cursor = self.cursor;
        self.pending = self
            .pending
            .take()
            .filter(|traversal| !was_dropped(cursor) && !was_dropped(traversal.origin))
            .map(|traversal| Traversal {
                origin: remap(traversal.origin),
                ..traversal
            });
        self.cursor = remap(cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::endpoint::ProfileId;

    fn remote() -> ClientEndpointId {
        ClientEndpointId::Ssh(ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap())
    }

    fn online() -> HashSet<ClientEndpointId> {
        [ClientEndpointId::Local, remote()].into_iter().collect()
    }

    #[test]
    fn visits_coalesce_and_traverse() {
        let mut history = History::default();
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
        history.observe(&ClientEndpointId::Local, "boot", None);
        history.observe(&remote(), "rboot", Some("p9"));
        assert_eq!(history.visits.len(), 2);

        let back = history.back(&online()).unwrap();
        assert_eq!(back.visit.pane_id, "p1");
        assert!(history.forward(&online()).is_some());
        assert!(history.forward(&online()).is_none());
        let back = history.back(&online()).unwrap();
        // Observing the target of an answered traversal confirms it without
        // a new visit.
        arrive(&mut history, &back);
        assert_eq!(history.visits.len(), 2);
        assert_eq!(history.cursor, 0);
        // A fresh visit truncates the forward list.
        history.observe(&ClientEndpointId::Local, "boot", Some("p2"));
        assert_eq!(history.visits.len(), 2);
        assert!(history.forward(&online()).is_none());
    }

    #[test]
    fn retirement_and_reboot_prune_entries_and_keep_the_cursor_sane() {
        let mut history = History::default();
        history.observe(&remote(), "rboot", Some("p9"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
        history.observe(&remote(), "rboot", Some("p8"));
        history.retire(&remote());
        assert_eq!(history.visits.len(), 1);
        assert_eq!(history.cursor, 0);
        history.observe(&ClientEndpointId::Local, "boot", Some("p2"));
        history.observe_boot(&ClientEndpointId::Local, "boot-2");
        assert!(history.visits.is_empty());
        assert!(history.back(&online()).is_none());
    }

    fn observe_local(history: &mut History, pane_id: &str) {
        history.observe(&ClientEndpointId::Local, "boot", Some(pane_id));
    }

    /// The endpoint answers the step's navigation, and then shows its target.
    fn arrive(history: &mut History, step: &Step) {
        history.traversal_answered(step, false);
        let visit = &step.visit;
        history.observe(&visit.endpoint_id, &visit.boot_id, Some(&visit.pane_id));
    }

    fn panes(history: &History) -> Vec<&str> {
        history
            .visits
            .iter()
            .map(|visit| visit.pane_id.as_str())
            .collect()
    }

    #[test]
    fn closed_panes_are_pruned_and_equal_neighbours_merge() {
        let mut history = History::default();
        history.observe(&remote(), "rboot", Some("p9"));
        for pane_id in ["p1", "p2", "p1", "p3"] {
            observe_local(&mut history, pane_id);
        }
        history.observe_panes(&ClientEndpointId::Local, "boot", ["p1", "p3"]);
        // The remote visit is untouched; p1 p2 p1 collapses into one p1.
        assert_eq!(panes(&history), ["p9", "p1", "p3"]);
        assert_eq!(history.cursor, 2);
        let step = history.back(&online()).unwrap();
        assert_eq!(step.visit.pane_id, "p1");

        // Closing the current pane moves the cursor to the nearest earlier survivor.
        arrive(&mut history, &step);
        history.observe_panes(&ClientEndpointId::Local, "boot", ["p3"]);
        assert_eq!(panes(&history), ["p9", "p3"]);
        assert_eq!(history.cursor, 0);
    }

    #[test]
    fn failed_traversal_rewinds_and_forgets_a_closed_target() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        let target = history.back(&online()).unwrap();
        assert_eq!(target.visit.pane_id, "p2");
        history.traversal_failed(&target, false);
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 2);

        let target = history.back(&online()).unwrap();
        history.traversal_failed(&target, true);
        assert_eq!(panes(&history), ["p1", "p3"]);
        assert_eq!(history.cursor, 1);
        assert_eq!(history.back(&online()).unwrap().visit.pane_id, "p1");
    }

    #[test]
    fn snapshots_on_the_traversed_path_do_not_truncate_forward_history() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        // Two quick steps back, then snapshots that predate and trail the
        // focus, before the second step's answer and after it.
        history.back(&online()).unwrap();
        let second = history.back(&online()).unwrap();
        observe_local(&mut history, "p3");
        observe_local(&mut history, "p2");
        history.traversal_answered(&second, false);
        observe_local(&mut history, "p2");
        observe_local(&mut history, "p1");
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_none());
        let target = history.forward(&online()).unwrap();
        assert_eq!(target.visit.pane_id, "p2");

        // Once the step was carried out, a snapshot from before it is still
        // recognised, and focus elsewhere is a real visit.
        history.traversal_answered(&target, false);
        observe_local(&mut history, "p1");
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert!(history.pending.is_some());
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p2", "p9"]);
        assert!(history.pending.is_none());
    }

    #[test]
    fn a_pane_seen_before_the_step_is_answered_is_not_a_visit() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        // Back to p2. Before the endpoint answers, a script focuses p9, or
        // the focused pane closes and the endpoint focuses p9 itself.
        let target = history.back(&online()).unwrap();
        observe_local(&mut history, "p9");
        observe_local(&mut history, "p8");
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 1);
        assert!(history.pending.is_some());

        // The step lands: nothing was lost on either side of it.
        arrive(&mut history, &target);
        assert!(history.pending.is_none());
        let forward = history.forward(&online()).unwrap();
        assert_eq!(forward.visit.pane_id, "p3");
        arrive(&mut history, &forward);
        let back = history.back(&online()).unwrap();
        assert_eq!(back.visit, target.visit);
        arrive(&mut history, &back);
        assert_eq!(history.back(&online()).unwrap().visit.pane_id, "p1");
    }

    #[test]
    fn a_pane_equal_to_the_target_seen_before_the_answer_confirms_nothing() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p1"] {
            observe_local(&mut history, pane_id);
        }
        // Two quick steps back: to `p2`, then to the first `p1`. The pane
        // still shown, the last `p1`, equals the second step's target.
        let first = history.back(&online()).unwrap();
        let second = history.back(&online()).unwrap();
        assert_eq!(second.visit.pane_id, "p1");
        observe_local(&mut history, "p1");
        assert!(history.pending.is_some());

        // Each step lands in turn, the first after the second was pressed.
        history.traversal_answered(&first, false);
        observe_local(&mut history, "p2");
        assert!(history.pending.is_some());
        arrive(&mut history, &second);

        assert_eq!(panes(&history), ["p1", "p2", "p1"]);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_none());
        assert_eq!(history.forward(&online()).unwrap().visit.pane_id, "p2");
    }

    #[test]
    fn a_target_shown_ahead_of_its_answer_ends_the_step_at_the_answer() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        // The endpoint sends the snapshot that shows `p2` before its answer.
        let step = history.back(&online()).unwrap();
        observe_local(&mut history, "p2");
        assert!(history.pending.is_some());
        history.traversal_answered(&step, true);
        assert!(history.pending.is_none());
        assert_eq!(history.cursor, 1);

        // No later snapshot is needed: the next pane is a visit after `p2`.
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p2", "p9"]);
    }

    #[test]
    fn an_earlier_steps_answer_or_failure_is_not_that_of_a_later_step_to_an_equal_pane() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p1", "p2"] {
            observe_local(&mut history, pane_id);
        }
        // Three quick steps back. The first and the third lead to equal
        // visits.
        let first = history.back(&online()).unwrap();
        let second = history.back(&online()).unwrap();
        let third = history.back(&online()).unwrap();
        assert_eq!(first.visit, third.visit);

        // The first step is answered with its `p1` shown, and the second
        // one fails: neither is the third step's.
        history.traversal_answered(&first, true);
        observe_local(&mut history, "p1");
        history.traversal_failed(&second, false);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_some());

        arrive(&mut history, &third);
        assert_eq!(panes(&history), ["p1", "p2", "p1", "p2"]);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_none());
        assert_eq!(history.forward(&online()).unwrap().visit.pane_id, "p2");
    }

    #[test]
    fn a_step_to_another_endpoint_keeps_the_forward_list_whatever_that_endpoint_had_focused() {
        let mut history = History::default();
        observe_local(&mut history, "p1");
        history.observe(&remote(), "rboot", Some("r5"));
        observe_local(&mut history, "p1");
        observe_local(&mut history, "p2");

        // Back to Local p1, then to the remote, each arriving.
        let step = history.back(&online()).unwrap();
        assert_eq!(step.visit.pane_id, "p1");
        arrive(&mut history, &step);
        let step = history.back(&online()).unwrap();
        assert_eq!(step.visit.pane_id, "r5");
        arrive(&mut history, &step);

        // While the client was away, a script focused p9 on Local. Back to
        // the first visit shows p9 before the step's own focus lands.
        let step = history.back(&online()).unwrap();
        assert_eq!(step.visit.pane_id, "p1");
        observe_local(&mut history, "p9");
        arrive(&mut history, &step);

        assert_eq!(panes(&history), ["p1", "r5", "p1", "p2"]);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_none());
        assert_eq!(history.forward(&online()).unwrap().visit.pane_id, "r5");
    }

    #[test]
    fn a_failed_step_returns_to_its_origin_whatever_was_seen_meanwhile() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        let target = history.back(&online()).unwrap();
        observe_local(&mut history, "p9");
        history.traversal_failed(&target, false);
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 2);
        assert!(history.pending.is_none());

        // With no traversal pending, the pane now shown is a visit as usual.
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p2", "p3", "p9"]);
    }

    #[test]
    fn only_the_pending_steps_own_answer_counts() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        // Two quick steps back: the first one's answer says nothing about
        // the second, which is still on its way.
        let first = history.back(&online()).unwrap();
        let second = history.back(&online()).unwrap();
        history.traversal_answered(&first, false);
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);

        // A carried-out step whose target never shows ends at the next pane
        // off its path, which follows the target.
        history.traversal_answered(&second, false);
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p9"]);
        assert_eq!(history.cursor, 1);
        assert!(history.pending.is_none());
    }

    #[test]
    fn offline_endpoints_are_skipped_and_capacity_is_bounded() {
        let mut history = History::default();
        history.observe(&remote(), "rboot", Some("p9"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p2"));
        let local_only: HashSet<_> = [ClientEndpointId::Local].into_iter().collect();
        assert_eq!(history.back(&local_only).unwrap().visit.pane_id, "p1");
        assert!(history.back(&local_only).is_none());

        let mut history = History::default();
        for index in 0..(CAPACITY + 10) {
            history.observe(&ClientEndpointId::Local, "boot", Some(&format!("p{index}")));
        }
        assert_eq!(history.visits.len(), CAPACITY);
        assert_eq!(history.cursor, CAPACITY - 1);
    }
}
