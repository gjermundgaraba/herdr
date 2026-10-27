//! What the fork keeps per client shell beside upstream's state, and the
//! fork's Navigator layouts.

#[derive(Default)]
pub(super) struct ForkState {
    /// Each endpoint's Back/Forward trail of focused panes.
    pub(super) history: super::history::Trails,
    /// The `ui.pick` picker this client shows.
    pub(super) picks: super::picks::ClientPicks,
}

/// Which rows the Navigator lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NavigatorLayout {
    /// Machines, workspaces, and their panes.
    Tree,
    /// Every agent pane across machines as a flat list in priority order.
    Agents,
    /// Workspaces alone; their panes appear only while searching.
    Workspaces,
}
