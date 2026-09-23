//! Bounded, coalesced Back/Forward history of pane visits across endpoints.
//!
//! A visit is recorded once per accepted active-endpoint snapshot whose focused
//! pane differs from the current entry, so rapid intermediate focus changes that
//! never produced a snapshot are not visits. Traversal moves the cursor
//! optimistically; the observation that lands on the traversal target is then
//! recognised instead of being pushed as a new visit, and a failed traversal
//! returns the cursor to where it started. Visits to panes an endpoint no longer
//! reports are pruned whenever that endpoint's snapshot is accepted.
use std::collections::{HashSet, VecDeque};

use crate::client::endpoint::ClientEndpointId;

const CAPACITY: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visit {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) boot_id: String,
    pub(crate) pane_id: String,
}

/// An in-flight traversal whose target is the cursor.
#[derive(Debug)]
struct Traversal {
    /// Index of the visit that was current before the traversal began.
    origin: usize,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    visits: VecDeque<Visit>,
    /// Index of the current visit; meaningful only while `visits` is non-empty.
    cursor: usize,
    /// Traversal whose arrival must not truncate the forward list.
    pending: Option<Traversal>,
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

    /// The focus request for a traversal failed: return to the visit it started
    /// from, and forget the target when its pane no longer exists.
    pub(crate) fn traversal_failed(&mut self, visit: &Visit, pane_closed: bool) {
        if self.current() == Some(visit) {
            if let Some(traversal) = self.pending.take() {
                self.cursor = traversal.origin;
            }
        }
        if pane_closed {
            self.rebuild(|candidate| candidate != visit);
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
    pub(crate) fn back(&mut self, online: &HashSet<ClientEndpointId>) -> Option<Visit> {
        let index = (0..self.cursor)
            .rev()
            .find(|index| online.contains(&self.visits[*index].endpoint_id))?;
        self.traverse(index)
    }

    /// Step to the nearest later visit on an online endpoint.
    pub(crate) fn forward(&mut self, online: &HashSet<ClientEndpointId>) -> Option<Visit> {
        let index = (self.cursor + 1..self.visits.len())
            .find(|index| online.contains(&self.visits[*index].endpoint_id))?;
        self.traverse(index)
    }

    fn traverse(&mut self, index: usize) -> Option<Visit> {
        let visit = self.visits.get(index)?.clone();
        // Chained steps keep the first origin so the whole path stays recognised.
        let origin = self
            .pending
            .as_ref()
            .map_or(self.cursor, |traversal| traversal.origin);
        self.cursor = index;
        self.pending = Some(Traversal { origin });
        Some(visit)
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
        assert_eq!(back.pane_id, "p1");
        assert!(history.forward(&online()).is_some());
        assert!(history.forward(&online()).is_none());
        assert!(history.back(&online()).is_some());
        // Observing the traversal target confirms it without a new visit.
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
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
        assert_eq!(history.back(&online()).unwrap().pane_id, "p1");

        // Closing the current pane moves the cursor to the nearest earlier survivor.
        observe_local(&mut history, "p1");
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
        assert_eq!(target.pane_id, "p2");
        history.traversal_failed(&target, false);
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 2);

        let target = history.back(&online()).unwrap();
        history.traversal_failed(&target, true);
        assert_eq!(panes(&history), ["p1", "p3"]);
        assert_eq!(history.cursor, 1);
        assert_eq!(history.back(&online()).unwrap().pane_id, "p1");
    }

    #[test]
    fn snapshots_on_the_traversed_path_do_not_truncate_forward_history() {
        let mut history = History::default();
        for pane_id in ["p1", "p2", "p3"] {
            observe_local(&mut history, pane_id);
        }
        // Two quick steps back, then snapshots that predate and trail the focus.
        history.back(&online()).unwrap();
        history.back(&online()).unwrap();
        observe_local(&mut history, "p3");
        observe_local(&mut history, "p2");
        observe_local(&mut history, "p1");
        assert_eq!(panes(&history), ["p1", "p2", "p3"]);
        assert_eq!(history.cursor, 0);
        assert!(history.pending.is_none());
        assert_eq!(history.forward(&online()).unwrap().pane_id, "p2");

        // Focus elsewhere during a traversal is a real visit.
        observe_local(&mut history, "p9");
        assert_eq!(panes(&history), ["p1", "p2", "p9"]);
        assert!(history.pending.is_none());
    }

    #[test]
    fn offline_endpoints_are_skipped_and_capacity_is_bounded() {
        let mut history = History::default();
        history.observe(&remote(), "rboot", Some("p9"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p1"));
        history.observe(&ClientEndpointId::Local, "boot", Some("p2"));
        let local_only: HashSet<_> = [ClientEndpointId::Local].into_iter().collect();
        assert_eq!(history.back(&local_only).unwrap().pane_id, "p1");
        assert!(history.back(&local_only).is_none());

        let mut history = History::default();
        for index in 0..(CAPACITY + 10) {
            history.observe(&ClientEndpointId::Local, "boot", Some(&format!("p{index}")));
        }
        assert_eq!(history.visits.len(), CAPACITY);
        assert_eq!(history.cursor, CAPACITY - 1);
    }
}
