//! The `priority` order of the Agents list: one machine's list, and the
//! cross-host list that the agent picker and keyboard agent navigation
//! share, which merges each endpoint's rows in that endpoint's own order.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/sidebar/agents/priority.rs` (see "Sibling fork:
//! Herdr GPUI" in `FORK.md`).

use super::*;

/// What the `priority` order compares an agent by: the daemon's
/// `priority_key`, as a plain byte string, smallest first. The key already
/// carries the priority lift and compares across daemons. An empty key,
/// which a daemon that sends no keys decodes to, follows every non-empty key.
/// A stable sort by it keeps equal keys in their snapshot order.
pub(in crate::client::shell) fn priority_key(agent: &ClientShellAgent) -> (bool, &[u8]) {
    (agent.priority_key.is_empty(), agent.priority_key.as_bytes())
}

/// The `priority` order across endpoints. A client-local rule comes first:
/// the rows of an endpoint that is not online follow every online row.
/// Within each of those halves [`priority_key`] decides. Equal keys keep the
/// rows' order, which is the endpoint display order, then the agents' order
/// within their endpoint, since the sort is stable.
pub(super) fn priority_order(
    left: &AggregateAgentRow<'_>,
    right: &AggregateAgentRow<'_>,
) -> std::cmp::Ordering {
    left.endpoint
        .stale()
        .cmp(&right.endpoint.stale())
        .then_with(|| priority_key(left.agent).cmp(&priority_key(right.agent)))
}
