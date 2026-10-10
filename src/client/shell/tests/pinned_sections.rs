//! Pinned sections come from the endpoint: `space_groups::sidebar_rows` turns
//! the snapshot's `pinned_sections` into header and mirror rows without
//! reading the spaces' tokens or the client's config.

use super::*;
use crate::client::shell::space_groups::{sidebar_rows, SidebarRow};
use crate::protocol::ClientShellPinnedSection;

fn spaces(count: usize) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let template = projected.workspaces[0].clone();
    projected.workspaces = (1..=count)
        .map(|number| ClientShellWorkspace {
            workspace_id: format!("ws_{number}"),
            number,
            label: format!("space-{number}"),
            focused: number == 1,
            ..template.clone()
        })
        .collect();
    projected
}

fn section(title: &str, fg: Option<&str>, workspace_ids: &[&str]) -> ClientShellPinnedSection {
    ClientShellPinnedSection {
        title: title.into(),
        fg: fg.map(str::to_owned),
        workspace_ids: workspace_ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// Rows as text: `# title` for a pinned header, `> id` for a mirror, `---`
/// for the divider, and the bare id for a space's own row.
fn described(snapshot: &ClientShellSnapshot, collapsed: &HashSet<String>) -> Vec<String> {
    sidebar_rows(snapshot, collapsed)
        .iter()
        .map(|row| match row {
            SidebarRow::Marker(marker) => match &marker.group {
                Some(group) if group.pinned => format!("# {}", group.name),
                Some(group) => format!("[{}]", group.name),
                None => "---".to_owned(),
            },
            SidebarRow::Mirror(entry) => {
                format!("> {}", snapshot.workspaces[entry.index].workspace_id)
            }
            SidebarRow::Workspace(entry) => snapshot.workspaces[entry.index].workspace_id.clone(),
        })
        .collect()
}

#[test]
fn a_snapshot_without_sections_has_no_pinned_rows() {
    let mut projected = spaces(2);
    // Tokens alone pin nothing: the endpoint decides.
    projected.workspaces[0].tokens = vec![("prio".into(), "1".into())];

    assert_eq!(described(&projected, &HashSet::new()), ["ws_1", "ws_2"]);
}

#[test]
fn sections_and_their_members_keep_the_order_the_endpoint_sent() {
    let mut projected = spaces(4);
    projected.pinned_sections = vec![
        section("Review", None, &["ws_4", "ws_1"]),
        section("Priority", Some("#e5c07b"), &["ws_3"]),
    ];

    assert_eq!(
        described(&projected, &HashSet::new()),
        [
            "# Review",
            "> ws_4",
            "> ws_1",
            "# Priority",
            "> ws_3",
            "---",
            "ws_1",
            "ws_2",
            "ws_3",
            "ws_4",
        ]
    );
}

#[test]
fn a_space_in_two_sections_is_mirrored_in_both_and_keeps_its_own_row() {
    let mut projected = spaces(2);
    projected.pinned_sections = vec![
        section("Priority", None, &["ws_2"]),
        section("Review", None, &["ws_1", "ws_2"]),
    ];

    assert_eq!(
        described(&projected, &HashSet::new()),
        [
            "# Priority",
            "> ws_2",
            "# Review",
            "> ws_1",
            "> ws_2",
            "---",
            "ws_1",
            "ws_2"
        ]
    );
}

#[test]
fn members_the_snapshot_does_not_list_are_skipped() {
    let mut projected = spaces(2);
    projected.pinned_sections = vec![
        section("Gone", None, &["ws_9"]),
        section("Review", None, &["ws_9", "ws_2"]),
    ];

    // A section left without a member is not shown at all.
    assert_eq!(
        described(&projected, &HashSet::new()),
        ["# Review", "> ws_2", "---", "ws_1", "ws_2"]
    );
}

#[test]
fn a_collapsed_section_keeps_its_header_and_members_but_shows_no_mirror() {
    let mut projected = spaces(3);
    projected.pinned_sections = vec![
        section("Priority", None, &["ws_1", "ws_3"]),
        section("Review", None, &["ws_2"]),
    ];
    let collapsed = HashSet::from(["pinned:Priority".to_owned()]);

    assert_eq!(
        described(&projected, &collapsed),
        [
            "# Priority",
            "# Review",
            "> ws_2",
            "---",
            "ws_1",
            "ws_2",
            "ws_3"
        ]
    );
    let rows = sidebar_rows(&projected, &collapsed);
    let SidebarRow::Marker(header) = &rows[0] else {
        panic!("pinned header");
    };
    assert!(header.group.as_ref().is_some_and(|group| group.collapsed));
    assert_eq!(header.members, [0, 2]);
}

#[test]
fn the_title_color_is_the_one_the_endpoint_sent() {
    let mut projected = spaces(1);
    projected.pinned_sections = vec![
        section("Long", Some("#e5c07b"), &["ws_1"]),
        section("Short", Some("#f80"), &["ws_1"]),
        section("Default", None, &["ws_1"]),
        section("Unreadable", Some("yellow"), &["ws_1"]),
    ];

    let colors = sidebar_rows(&projected, &HashSet::new())
        .iter()
        .filter_map(|row| match row {
            SidebarRow::Marker(marker) => marker.group.as_ref(),
            _ => None,
        })
        .filter(|group| group.pinned)
        .map(|group| group.title_color)
        .collect::<Vec<_>>();

    assert_eq!(
        colors,
        [
            Some(ratatui::style::Color::Rgb(0xe5, 0xc0, 0x7b)),
            Some(ratatui::style::Color::Rgb(0xff, 0x88, 0x00)),
            None,
            // A color this client cannot read falls back to its default.
            None,
        ]
    );
}

#[test]
fn pinned_sections_precede_the_groups_of_the_regular_list() {
    let mut projected = spaces(3);
    for index in [1, 2] {
        projected.workspaces[index].tokens = vec![("space_group".into(), "work".into())];
    }
    projected.pinned_sections = vec![section("Priority", None, &["ws_3"])];

    assert_eq!(
        described(&projected, &HashSet::new()),
        [
            "# Priority",
            "> ws_3",
            "---",
            "ws_1",
            "[work]",
            "ws_2",
            "ws_3"
        ]
    );
}
