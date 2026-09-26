//! The fork's `priority` grouping and age direction, shared by server and client sorts.
//!
//! Priority spaces: a space with a `space_priority` workspace metadata token
//! lifts its blocked and done agents ahead of every other agent. The
//! space-priority plugin sets the token; its value doubles as a sidebar
//! marker. Working, idle, and unknown agents in a priority space keep their
//! normal place, and the `spaces` order ignores it.
//!
//! Blocked and done agents are waiting on the user, so they queue oldest
//! first; working and idle agents show the newest first. The server uses
//! lifecycle sequences; built-in client lists use observation and manual requeue order.

use crate::api::schema::AgentStatus;
use std::cmp::Reverse;
use std::collections::HashSet;

/// Workspace metadata key that marks a priority space.
pub(crate) const TOKEN: &str = "space_priority";

/// Sort key for an agent in the `priority` order. `marked` says whether its
/// space carries `TOKEN`, and `recency` grows with each ordering event.
pub(crate) fn sort_key(
    marked: bool,
    status: AgentStatus,
    recency: u64,
) -> (Reverse<bool>, Reverse<u8>, u64) {
    let waiting = matches!(status, AgentStatus::Blocked | AgentStatus::Done);
    let age = if waiting { recency } else { u64::MAX - recency };
    (Reverse(marked && waiting), Reverse(rank(status)), age)
}

/// Status rank for the priority order, most urgent highest. The fork owns
/// this order, so it keeps its own copy instead of depending on client code.
fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 4,
        AgentStatus::Done => 3,
        AgentStatus::Working => 2,
        AgentStatus::Idle => 1,
        AgentStatus::Unknown => 0,
    }
}

/// Ids of the priority spaces in a client snapshot.
pub(crate) fn marked_workspaces(snapshot: &crate::protocol::ClientShellSnapshot) -> HashSet<&str> {
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace.tokens.iter().any(|(key, _)| key == TOKEN))
        .map(|workspace| workspace.workspace_id.as_str())
        .collect()
}
