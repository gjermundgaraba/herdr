//! The fork's `priority` agent order, shared by server and client sorts.
//!
//! Blocked and done agents are waiting on the user, so they queue oldest
//! first; working and idle agents show the newest first. The server computes
//! the order and sends each agent's place in it as `priority_key`, so clients
//! sort and merge endpoints without knowing the rule. Built-in TUI lists still
//! use their own observation and manual requeue order.
//!
//! `ui.agent_priority_tokens` lists workspace metadata token keys. A space that
//! carries any of them with a non-empty value is lifted: its blocked and done
//! agents come before every other agent. Its working, idle, and unknown agents
//! keep their normal place, and the `spaces` order ignores the tokens. The
//! value of the first listed token a space carries marks its agents in the
//! agent picker.

use crate::api::schema::AgentStatus;
use std::collections::HashSet;

/// Sort key for an agent in the `priority` order, `(tier, rank, age)`
/// ascending: tier is `0` for a lifted waiting agent and `1` otherwise.
/// `lifted` says whether its space carries a priority token, and `recency`
/// grows with each ordering event.
pub(crate) fn sort_key(lifted: bool, status: AgentStatus, recency: u64) -> (u8, u8, u64) {
    let waiting = matches!(status, AgentStatus::Blocked | AgentStatus::Done);
    let age = if waiting { recency } else { u64::MAX - recency };
    (u8::from(!(lifted && waiting)), rank(status), age)
}

/// The wire form of [`sort_key`], sent as `priority_key` in client snapshots:
/// `<tier>.<rank>.<age>`, where tier is `0` for a lifted waiting agent and `1`
/// otherwise, rank counts down from blocked (`0`) to unknown (`4`), and age is
/// 16 hex digits. Every key has the same length, so byte order is key order.
///
/// `stamp` comes from [`next_stamp`], which makes keys from different daemons
/// comparable. Equal keys are possible (agents with no ordering event since
/// their daemon started all have stamp 0); a client breaks such a tie by its
/// own endpoint order, then by the agents' order in their snapshot.
pub(crate) fn priority_key(lifted: bool, status: AgentStatus, stamp: u64) -> String {
    let (tier, rank, age) = sort_key(lifted, status, stamp);
    format!("{tier}.{rank}.{age:016x}")
}

/// The next stamp of the clock that orders agents within a priority group.
///
/// A daemon's lifecycle sequences count only its own changes, so they cannot
/// order agents of different daemons. Stamps are wall-clock microseconds since
/// the Unix epoch instead, pushed forward to stay strictly increasing within
/// one daemon whatever its clock does. Across daemons they order events as
/// well as the hosts' clocks agree.
pub(crate) fn next_stamp(previous: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        });
    now.max(previous.saturating_add(1))
}

/// Status rank for the priority order, most urgent first. The fork owns
/// this order, so it keeps its own copy instead of depending on client code.
fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Done => 1,
        AgentStatus::Working => 2,
        AgentStatus::Idle => 3,
        AgentStatus::Unknown => 4,
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
    fn wire_keys_order_like_sort_keys() {
        let statuses = [
            AgentStatus::Blocked,
            AgentStatus::Done,
            AgentStatus::Working,
            AgentStatus::Idle,
            AgentStatus::Unknown,
        ];
        let stamps = [0, 1, 9, 1_790_000_000_000_000, u64::MAX];
        let mut cases = Vec::new();
        for lifted in [false, true] {
            for status in statuses {
                for stamp in stamps {
                    cases.push((lifted, status, stamp));
                }
            }
        }
        for &(lifted, status, stamp) in &cases {
            let key = priority_key(lifted, status, stamp);
            assert_eq!(key.len(), 20, "{key}");
            for &(other_lifted, other_status, other_stamp) in &cases {
                assert_eq!(
                    key.cmp(&priority_key(other_lifted, other_status, other_stamp)),
                    sort_key(lifted, status, stamp).cmp(&sort_key(
                        other_lifted,
                        other_status,
                        other_stamp
                    )),
                    "{lifted} {status:?} {stamp} vs {other_lifted} {other_status:?} {other_stamp}"
                );
            }
        }
    }

    #[test]
    fn wire_key_spells_tier_rank_and_age() {
        assert_eq!(
            priority_key(true, AgentStatus::Done, 0x2a),
            "0.1.000000000000002a"
        );
        // Working and idle agents show the newest first, so their age inverts.
        assert_eq!(
            priority_key(true, AgentStatus::Working, 0x2a),
            "1.2.ffffffffffffffd5"
        );
        assert_eq!(
            priority_key(false, AgentStatus::Blocked, 0),
            "1.0.0000000000000000"
        );
    }

    #[test]
    fn stamps_never_repeat_or_go_back() {
        let first = next_stamp(0);
        assert!(first > 1_700_000_000_000_000, "{first}");
        assert_eq!(next_stamp(u64::MAX - 1), u64::MAX);
        let ahead = first + 3_600_000_000;
        assert_eq!(next_stamp(ahead), ahead + 1);
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
