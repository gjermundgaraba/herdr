//! Space groups and worktree families over the server's space order.
//!
//! A repo with an open checkout and at least one other open space forms a
//! family whose spaces share the checkout's group. The `space_group` workspace
//! metadata token on that checkout, the family anchor, names the group. The
//! sidebar shows spaces in server order and starts a header wherever the group
//! changes, so keeping each group together is a matter of where spaces move.

/// Workspace metadata key that names a space's group.
pub(crate) const GROUP_TOKEN: &str = "space_group";

/// What ordering needs to know about one space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Space<'a> {
    /// Repo key and whether the space is a linked worktree.
    pub(crate) worktree: Option<(&'a str, bool)>,
    /// The space's own `space_group` token.
    pub(crate) group: Option<&'a str>,
}

/// The index of the space whose group `index` shares: the first non-linked
/// checkout of a repo with at least two open spaces, or `index` itself.
pub(crate) fn anchor(spaces: &[Space<'_>], index: usize) -> usize {
    let Some((key, _)) = spaces[index].worktree else {
        return index;
    };
    let mut members = spaces
        .iter()
        .enumerate()
        .filter(|(_, space)| space.worktree.is_some_and(|(other, _)| other == key));
    let mut checkout = None;
    let mut count = 0;
    for (member, space) in members.by_ref() {
        count += 1;
        if checkout.is_none() && space.worktree.is_some_and(|(_, linked)| !linked) {
            checkout = Some(member);
        }
    }
    match checkout {
        Some(checkout) if count >= 2 => checkout,
        _ => index,
    }
}

/// The group of the family `index` belongs to.
pub(crate) fn group<'a>(spaces: &[Space<'a>], index: usize) -> Option<&'a str> {
    spaces[anchor(spaces, index)]
        .group
        .filter(|group| !group.is_empty())
}

/// Every space that shares `anchor_index`'s group, the anchor first, then
/// server order.
pub(crate) fn family(spaces: &[Space<'_>], anchor_index: usize) -> Vec<usize> {
    let mut family = vec![anchor_index];
    family.extend(
        (0..spaces.len())
            .filter(|index| *index != anchor_index && anchor(spaces, *index) == anchor_index),
    );
    family
}

/// Top-level units in sidebar order, each by its first member: a family shows
/// where its first member is, wherever its other members sit.
fn units(spaces: &[Space<'_>]) -> Vec<usize> {
    let mut anchors = Vec::new();
    let mut units = Vec::new();
    for index in 0..spaces.len() {
        let anchor = anchor(spaces, index);
        if !anchors.contains(&anchor) {
            anchors.push(anchor);
            units.push(index);
        }
    }
    units
}

/// Where `family` goes when it moves into `group`: right after the group's
/// last unit in sidebar order, or, for a new group, above the first ungrouped
/// unit. Leaving every group moves the family to the end. The result is the
/// space to move the family before, `None` for the end; the outer `None` means
/// it is already in place.
pub(crate) fn placement(
    spaces: &[Space<'_>],
    family: &[usize],
    group: Option<&str>,
) -> Option<Option<usize>> {
    let rest = units(spaces)
        .into_iter()
        .filter(|unit| !family.contains(unit))
        .collect::<Vec<_>>();
    let at = match group {
        None => rest.len(),
        Some(group) => rest
            .iter()
            .rposition(|unit| self::group(spaces, *unit) == Some(group))
            .map(|last| last + 1)
            .or_else(|| {
                rest.iter()
                    .position(|unit| self::group(spaces, *unit).is_none())
            })
            .unwrap_or(rest.len()),
    };
    moved(spaces.len(), family, rest.get(at).copied())
}

/// Where `family` goes to sit together at its member `at`, with its anchor
/// first: the space to move it before, `None` for the end; the outer `None`
/// means it is already together. A family gathers where it shows, at its first
/// member, unless a move placed its anchor.
pub(crate) fn gathering(
    spaces: &[Space<'_>],
    family: &[usize],
    at: usize,
) -> Option<Option<usize>> {
    let before = (at..spaces.len()).find(|index| !family.contains(index));
    moved(spaces.len(), family, before)
}

/// `Some(before)` when moving `family` as one block before `before`, or to the
/// end, changes the order of `len` spaces.
fn moved(len: usize, family: &[usize], before: Option<usize>) -> Option<Option<usize>> {
    let mut order = (0..len)
        .filter(|index| !family.contains(index))
        .collect::<Vec<_>>();
    let at = before
        .and_then(|before| order.iter().position(|index| *index == before))
        .unwrap_or(order.len());
    order.splice(at..at, family.iter().copied());
    (!order.into_iter().eq(0..len)).then_some(before)
}

/// The group the family of `anchor_index` takes after a move left it where it
/// now sits, when that differs from its current group. Landing inside a run
/// joins that run's group, or leaves every group inside the ungrouped spaces.
/// At a run boundary it keeps its group if a neighbour shares it or it is the
/// group's only family, and otherwise joins the run above it.
pub(crate) fn regroup<'a>(spaces: &[Space<'a>], anchor_index: usize) -> Option<Option<&'a str>> {
    let units = units(spaces);
    let position = units
        .iter()
        .position(|unit| anchor(spaces, *unit) == anchor_index)?;
    let groups = units
        .iter()
        .map(|unit| group(spaces, *unit))
        .collect::<Vec<_>>();
    let current = groups[position];
    let previous = position.checked_sub(1).map(|position| groups[position]);
    let next = groups.get(position + 1).copied();
    let only_family =
        current.is_some() && groups.iter().filter(|group| **group == current).count() == 1;
    let target = match (previous, next) {
        (Some(previous), Some(next)) if previous == next => previous,
        _ if previous.flatten() == current || next.flatten() == current || only_family => current,
        _ => previous.unwrap_or_else(|| next.flatten()),
    };
    (target != current).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space<'a>(group: Option<&'a str>, repo: Option<(&'a str, bool)>) -> Space<'a> {
        Space {
            worktree: repo,
            group,
        }
    }

    fn grouped<'a>(groups: &[Option<&'a str>]) -> Vec<Space<'a>> {
        groups.iter().map(|group| space(*group, None)).collect()
    }

    #[test]
    fn a_linked_child_resolves_to_its_checkout_family() {
        let spaces = [
            space(None, Some(("repo", true))),
            space(None, None),
            space(None, Some(("repo", false))),
            space(None, Some(("solo", true))),
        ];
        assert_eq!(anchor(&spaces, 0), 2);
        assert_eq!(anchor(&spaces, 3), 3);
        assert_eq!(family(&spaces, 2), [2, 0]);
        assert_eq!(family(&spaces, 1), [1]);
    }

    #[test]
    fn a_family_takes_its_checkouts_group() {
        let spaces = [
            space(Some("a"), Some(("repo", false))),
            space(Some("b"), Some(("repo", true))),
            space(Some(""), None),
        ];
        assert_eq!(group(&spaces, 1), Some("a"));
        assert_eq!(group(&spaces, 2), None);
    }

    #[test]
    fn joining_a_group_lands_after_its_last_member() {
        let spaces = grouped(&[Some("a"), Some("b"), None, None]);
        assert_eq!(placement(&spaces, &[3], Some("a")), Some(Some(1)));
    }

    #[test]
    fn joining_a_group_lands_after_its_last_family_where_it_shows() {
        let (a, b) = (Some("a"), Some("b"));
        // The worktree shows under its checkout, so the group ends at 0.
        let spaces = [
            space(a, Some(("repo", false))),
            space(b, None),
            space(b, None),
            space(None, Some(("repo", true))),
            space(None, None),
        ];
        assert_eq!(placement(&spaces, &[4], a), Some(Some(1)));
    }

    #[test]
    fn joining_a_group_brings_the_familys_stray_worktree_along() {
        let (a, b) = (Some("a"), Some("b"));
        let spaces = [
            space(a, None),
            space(None, Some(("repo", false))),
            space(b, None),
            space(None, Some(("repo", true))),
        ];
        assert_eq!(placement(&spaces, &[1, 3], a), Some(Some(2)));
    }

    #[test]
    fn a_family_gathers_where_it_shows_with_its_checkout_first() {
        let spaces = [
            space(None, Some(("repo", true))),
            space(None, None),
            space(None, Some(("repo", false))),
            space(None, Some(("repo", true))),
        ];
        let family = family(&spaces, 2);
        assert_eq!(gathering(&spaces, &family, 0), Some(Some(1)));
        // A move that placed the checkout gathers the family there.
        assert_eq!(gathering(&spaces, &family, 2), Some(None));
        let together = [
            space(None, Some(("repo", false))),
            space(None, Some(("repo", true))),
            space(None, None),
        ];
        assert_eq!(gathering(&together, &[0, 1], 0), None);
    }

    #[test]
    fn a_new_group_goes_above_the_ungrouped_spaces() {
        let spaces = grouped(&[Some("a"), None, None]);
        assert_eq!(placement(&spaces, &[2], Some("new")), Some(Some(1)));
    }

    #[test]
    fn leaving_moves_the_family_to_the_end_and_unchanged_is_none() {
        let spaces = [
            space(Some("a"), Some(("repo", false))),
            space(None, Some(("repo", true))),
            space(None, None),
        ];
        assert_eq!(placement(&spaces, &[0, 1], None), Some(None));
        assert_eq!(placement(&spaces, &[0, 1], Some("a")), None);
    }

    fn regrouped<'a>(groups: &[Option<&'a str>], moved: usize) -> Option<Option<&'a str>> {
        regroup(&grouped(groups), moved)
    }

    #[test]
    fn landing_inside_a_run_joins_its_group() {
        let (a, b) = (Some("a"), Some("b"));
        assert_eq!(regrouped(&[a, b, a, b], 1), Some(a));
        assert_eq!(regrouped(&[a, None, a, None], 2), Some(None));
        assert_eq!(regrouped(&[a, None, a], 1), Some(a));
    }

    #[test]
    fn a_boundary_keeps_a_shared_group_and_otherwise_joins_the_run_above() {
        let (a, b, c) = (Some("a"), Some("b"), Some("c"));
        // The end of its own run, or the start of the ungrouped spaces.
        assert_eq!(regrouped(&[a, a, b], 1), None);
        assert_eq!(regrouped(&[a, None, None], 1), None);
        // Separated from its group between two others: the run above wins.
        assert_eq!(regrouped(&[a, c, b, c], 1), Some(a));
        // At the top there is no run above, so it joins the run below.
        assert_eq!(regrouped(&[c, a, c], 0), Some(a));
    }

    #[test]
    fn a_group_of_one_family_moves_whole() {
        let (a, b, solo) = (Some("a"), Some("b"), Some("solo"));
        assert_eq!(regrouped(&[a, solo, b], 1), None);
        assert_eq!(regrouped(&[None, solo], 1), None);
    }

    #[test]
    fn a_whole_group_dropped_inside_another_stays_together() {
        let (a, b) = (Some("a"), Some("b"));
        let order = [b, a, a, b];
        assert_eq!(regrouped(&order, 1), None);
        assert_eq!(regrouped(&order, 2), None);
    }

    #[test]
    fn the_placement_it_makes_itself_is_stable() {
        let a = Some("a");
        // Joining lands after the last member; leaving lands at the end.
        assert_eq!(regrouped(&[a, a, None], 1), None);
        assert_eq!(regrouped(&[a, Some("b"), None], 2), None);
    }

    #[test]
    fn a_family_is_judged_by_its_checkout() {
        let spaces = [
            space(Some("a"), None),
            space(None, Some(("repo", false))),
            space(Some("a"), None),
            space(None, Some(("repo", true))),
        ];
        assert_eq!(regroup(&spaces, 1), Some(Some("a")));
    }
}
