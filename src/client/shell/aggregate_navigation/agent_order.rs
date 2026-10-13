//! The order of the cross-host Agents list, which the agent picker and
//! keyboard agent navigation share: each endpoint's rows numbered in its
//! snapshot's order, or its legacy view's, then the active endpoint's plugin
//! view, or else under the `priority` sort the daemons' `priority_key`s.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/sidebar/agents/priority.rs` (see "Sibling fork:
//! Herdr GPUI" in `FORK.md`).

use super::*;

/// One endpoint's rows in their unsorted order.
pub(super) fn endpoint_rows<'a>(
    endpoint: CachedEndpointSnapshot<'a>,
    agents: impl IntoIterator<Item = &'a ClientShellAgent>,
) -> impl Iterator<Item = AggregateAgentRow<'a>> {
    agents
        .into_iter()
        .enumerate()
        .map(move |(position, agent)| AggregateAgentRow {
            endpoint,
            agent,
            position,
        })
}

/// A legacy view supplies membership and an initial order. Without its
/// definition, the selected built-in sort determines the combined order.
pub(super) fn legacy_projected_agents(snapshot: &ClientShellSnapshot) -> Vec<&ClientShellAgent> {
    if snapshot.agent_view_label.is_some() {
        snapshot
            .agent_order
            .iter()
            .filter_map(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| &agent.pane_id == pane_id)
            })
            .collect()
    } else {
        snapshot.agents.iter().collect()
    }
}

/// The label of the plugin agent view the active endpoint applies, which
/// orders the Agents list across endpoints.
pub(in crate::client::shell) fn active_agent_view_label<'a>(
    endpoints: &'a [ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
) -> Option<&'a str> {
    cached_endpoint_snapshots(endpoints)
        .find(|endpoint| endpoint.endpoint_id == active_endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.agent_view_label.as_deref())
}

/// The `priority` order across endpoints. Client-local rules come first: the
/// rows of an endpoint that is not online follow every online row. Within
/// each of those halves the daemons' `priority_key`s decide, compared as
/// plain byte strings, smallest first; they already carry the priority lift
/// and compare across daemons. An empty key, which a daemon that sends no
/// keys decodes to, follows every non-empty key. Equal keys keep the endpoint
/// display order, then the agents' order within their endpoint.
pub(super) fn priority_order(
    left: &AggregateAgentRow<'_>,
    right: &AggregateAgentRow<'_>,
) -> std::cmp::Ordering {
    left.endpoint
        .stale()
        .cmp(&right.endpoint.stale())
        .then_with(|| {
            let (left, right) = (&left.agent.priority_key, &right.agent.priority_key);
            left.is_empty()
                .cmp(&right.is_empty())
                .then_with(|| left.as_bytes().cmp(right.as_bytes()))
        })
        .then_with(|| {
            left.endpoint
                .endpoint_index
                .cmp(&right.endpoint.endpoint_index)
        })
        .then_with(|| left.position.cmp(&right.position))
}
