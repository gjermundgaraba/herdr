//! What the fork keeps per client shell beside upstream's state.

#[derive(Default)]
pub(super) struct ForkState {
    /// Each endpoint's Back/Forward trail of focused panes.
    pub(super) history: super::history::Trails,
    /// Jumps daemons asked this client to perform.
    pub(super) client_jumps: super::client_jump::ClientJumps,
    /// The `ui.pick` picker this client shows.
    pub(super) picks: super::picks::ClientPicks,
    /// A press on a space group header, until it drags or toggles.
    pub(super) group_press: Option<super::space_groups::ClientGroupPress>,
}
