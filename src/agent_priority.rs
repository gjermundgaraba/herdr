//! The fork's `priority` agent order, shared by server and client sorts.
//!
//! Blocked and done agents are waiting on the user, so they queue oldest
//! first; working and idle agents show the newest first. The server uses
//! lifecycle sequences; built-in client lists use observation and manual requeue order.
//!
//! `ui.agent_priority_tokens` lists workspace metadata token keys. A space that
//! carries any of them with a non-empty value is lifted: its blocked and done
//! agents come before every other agent. Its working, idle, and unknown agents
//! keep their normal place, and the `spaces` order ignores the tokens. The
//! value of the first listed token a space carries marks its agents in the
//! agent picker.

use crate::api::schema::AgentStatus;
use std::cmp::Reverse;
use std::collections::HashSet;

/// Sort key for an agent in the `priority` order. `lifted` says whether its
/// space carries a priority token, and `recency` grows with each ordering
/// event.
pub(crate) fn sort_key(
    lifted: bool,
    status: AgentStatus,
    recency: u64,
) -> (Reverse<bool>, Reverse<u8>, u64) {
    let waiting = matches!(status, AgentStatus::Blocked | AgentStatus::Done);
    let age = if waiting { recency } else { u64::MAX - recency };
    (Reverse(lifted && waiting), Reverse(rank(status)), age)
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

/// A space's mark: the value of the first token in `priority_tokens` that
/// `value` finds with a non-empty value.
pub(crate) fn mark<'a>(
    priority_tokens: &[String],
    value: impl Fn(&str) -> Option<&'a str>,
) -> Option<&'a str> {
    priority_tokens
        .iter()
        .find_map(|key| value(key).filter(|value| !value.is_empty()))
}

/// Whether a space carries any token in `priority_tokens` with a non-empty
/// value.
pub(crate) fn lifted<'a>(
    priority_tokens: &[String],
    value: impl Fn(&str) -> Option<&'a str>,
) -> bool {
    mark(priority_tokens, value).is_some()
}

/// The mark of a space in a client snapshot.
pub(crate) fn workspace_mark<'a>(
    priority_tokens: &[String],
    workspace: &'a crate::protocol::ClientShellWorkspace,
) -> Option<&'a str> {
    mark(priority_tokens, |key| {
        workspace
            .tokens
            .iter()
            .find(|(token, _)| token == key)
            .map(|(_, value)| value.as_str())
    })
}

/// Ids of the lifted spaces in a client snapshot.
pub(crate) fn lifted_workspaces<'a>(
    priority_tokens: &[String],
    snapshot: &'a crate::protocol::ClientShellSnapshot,
) -> HashSet<&'a str> {
    if priority_tokens.is_empty() {
        return HashSet::new();
    }
    snapshot
        .workspaces
        .iter()
        .filter(|workspace| workspace_mark(priority_tokens, workspace).is_some())
        .map(|workspace| workspace.workspace_id.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup<'a>(tokens: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<&'a str> {
        move |key| {
            tokens
                .iter()
                .find(|(token, _)| *token == key)
                .map(|(_, value)| *value)
        }
    }

    #[test]
    fn mark_uses_the_first_configured_token_with_a_value() {
        let configured = ["lift".to_string(), "urgent".to_string()];
        assert_eq!(mark(&configured, lookup(&[("urgent", "!")])), Some("!"));
        assert_eq!(
            mark(&configured, lookup(&[("urgent", "!"), ("lift", "★")])),
            Some("★")
        );
        assert_eq!(
            mark(&configured, lookup(&[("lift", ""), ("urgent", "!")])),
            Some("!")
        );
        assert_eq!(mark(&configured, lookup(&[("other", "x")])), None);
        assert!(!lifted(&configured, lookup(&[("lift", "")])));
    }

    #[test]
    fn empty_token_list_lifts_and_marks_nothing() {
        let tokens = [("lift", "★")];
        assert_eq!(mark(&[], lookup(&tokens)), None);
        assert!(!lifted(&[], lookup(&tokens)));
    }

    #[test]
    fn lift_applies_only_to_waiting_agents() {
        let waiting = sort_key(true, AgentStatus::Done, 9);
        let other_blocked = sort_key(false, AgentStatus::Blocked, 1);
        let lifted_working = sort_key(true, AgentStatus::Working, 1);
        assert!(waiting < other_blocked);
        assert!(other_blocked < lifted_working);
    }
}
