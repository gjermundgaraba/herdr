//! Bounded, coalesced Back/Forward history of pane visits across endpoints.
//!
//! A visit is recorded once per accepted active-endpoint snapshot whose focused
//! pane differs from the current entry, so rapid intermediate focus changes that
//! never produced a snapshot are not visits. Traversal moves the cursor
//! optimistically; the observation that lands on the traversal target is then
//! recognised instead of being pushed as a new visit.
use std::collections::{HashSet, VecDeque};

use crate::client::endpoint::ClientEndpointId;

const CAPACITY: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visit {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) boot_id: String,
    pub(crate) pane_id: String,
}

#[derive(Debug, Default)]
pub(crate) struct History {
    visits: VecDeque<Visit>,
    /// Index of the current visit; meaningful only while `visits` is non-empty.
    cursor: usize,
    /// Traversal target whose arrival must not truncate the forward list.
    pending: Option<Visit>,
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
        if self.pending.as_ref() == Some(&visit) {
            self.pending = None;
            return;
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
        self.retain(|visit| &visit.endpoint_id != endpoint_id || visit.boot_id == boot_id);
    }

    pub(crate) fn retire(&mut self, endpoint_id: &ClientEndpointId) {
        self.retain(|visit| &visit.endpoint_id != endpoint_id);
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
        self.cursor = index;
        self.pending = Some(visit.clone());
        Some(visit)
    }

    fn current(&self) -> Option<&Visit> {
        self.visits.get(self.cursor)
    }

    fn retain(&mut self, keep: impl Fn(&Visit) -> bool) {
        let removed_before_cursor = self
            .visits
            .iter()
            .take(self.cursor)
            .filter(|visit| !keep(visit))
            .count();
        self.visits.retain(|visit| keep(visit));
        self.cursor = self
            .cursor
            .saturating_sub(removed_before_cursor)
            .min(self.visits.len().saturating_sub(1));
        if self.pending.as_ref().is_some_and(|visit| !keep(visit)) {
            self.pending = None;
        }
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
