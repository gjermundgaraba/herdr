//! The fork's `priority` agent order, which the server owns.
//!
//! Blocked and done agents are waiting on the user, so they queue oldest
//! first; working and idle agents show the newest first. The server computes
//! the order and sends each agent's place in it as `priority_key`, so clients
//! sort and merge endpoints without knowing the rule
//! (`client/shell/aggregate_navigation.rs`).
//!
//! A space whose [`PRIORITY_TOKEN`] workspace metadata token has a non-empty
//! value is lifted: its blocked and done agents come before every other agent.
//! Its working, idle, and unknown agents keep their normal place, and the
//! `spaces` order ignores the token. The token's value marks the space's
//! agents in the agent picker.

use crate::api::schema::AgentStatus;

/// Workspace metadata key that lifts a space in the priority order and marks
/// its agents. The space-priority plugin sets it.
pub(crate) const PRIORITY_TOKEN: &str = "space_priority";

/// Sort key for an agent in the `priority` order, `(tier, rank, age)`
/// ascending: tier is `0` for a lifted waiting agent and `1` otherwise.
/// `lifted` says whether its space carries the priority token, and `recency`
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

/// A space's mark: its [`PRIORITY_TOKEN`] value, unless that is empty.
pub(crate) fn mark(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// Whether a space whose [`PRIORITY_TOKEN`] is `value` is lifted.
pub(crate) fn lifted(value: Option<&str>) -> bool {
    mark(value).is_some()
}

/// The mark of a space in a client snapshot.
pub(crate) fn workspace_mark(workspace: &crate::protocol::ClientShellWorkspace) -> Option<&str> {
    mark(
        workspace
            .tokens
            .iter()
            .find(|(key, _)| key == PRIORITY_TOKEN)
            .map(|(_, value)| value.as_str()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_non_empty_priority_token_marks_and_lifts() {
        assert_eq!(mark(Some("★")), Some("★"));
        assert_eq!(mark(Some("")), None);
        assert_eq!(mark(None), None);
        assert!(lifted(Some("!")));
        assert!(!lifted(Some("")));
        assert!(!lifted(None));
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
