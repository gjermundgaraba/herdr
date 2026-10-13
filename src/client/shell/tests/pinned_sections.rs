//! Pinned sections come from the client's own config: `space_groups::sidebar_rows`
//! matches each `[[ui.sidebar.spaces.pinned]]` section against the snapshot's
//! spaces and their metadata tokens, for every endpoint alike.

use super::*;
use crate::client::shell::space_groups::{sidebar_rows, SidebarRow};
use crate::config::PinnedSpaceSection;

/// Spaces `ws_1`, `ws_2`, ... carrying the given tokens, in that order.
fn spaces(tokens: &[&[(&str, &str)]]) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let template = projected.workspaces[0].clone();
    projected.workspaces = tokens
        .iter()
        .enumerate()
        .map(|(index, tokens)| ClientShellWorkspace {
            workspace_id: format!("ws_{}", index + 1),
            number: index + 1,
            label: format!("space-{}", index + 1),
            focused: index == 0,
            tokens: tokens
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            ..template.clone()
        })
        .collect();
    projected
}

fn pinned(sections: &str) -> Vec<PinnedSpaceSection> {
    toml::from_str::<crate::config::SpacesSidebarConfig>(sections)
        .expect("pinned sections")
        .pinned
}

/// Rows as text: `# title` for a pinned header, `> id` for a mirror, `---`
/// for the divider, `[name]` for a group header, and the bare id for a
/// space's own row.
fn described(
    snapshot: &ClientShellSnapshot,
    pinned: &[PinnedSpaceSection],
    collapsed: &HashSet<String>,
) -> Vec<String> {
    sidebar_rows(snapshot, pinned, collapsed)
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
fn without_configured_sections_tokens_pin_nothing() {
    let projected = spaces(&[&[("prio", "1")], &[]]);

    assert_eq!(
        described(&projected, &[], &HashSet::new()),
        ["ws_1", "ws_2"]
    );
}

#[test]
fn sections_match_tokens_in_config_and_space_order() {
    let mut projected = spaces(&[
        &[("prio", "high")],
        &[("prio", ""), ("team", "web")],
        &[("prio", "low")],
        &[("prio", "highest"), ("team", "platform-eu")],
    ]);
    let sections = pinned(
        r##"
        [[pinned]]
        title = "Any priority"
        token = "prio"

        [[pinned]]
        title = "Exactly high"
        token = "prio"
        equals = "high"

        [[pinned]]
        title = "Platform"
        token = "team"
        starts_with = "platform"

        [[pinned]]
        title = "Mentions eb"
        token = "team"
        contains = "eb"

        [[pinned]]
        title = "Nobody"
        token = "owner"

        [[pinned]]
        title = "No such value"
        token = "prio"
        equals = "urgent"
        "##,
    );

    assert_eq!(
        described(&projected, &sections, &HashSet::new()),
        [
            // An empty value pins nothing; members keep the space order.
            "# Any priority",
            "> ws_1",
            "> ws_3",
            "> ws_4",
            // `equals` is the whole value, not a prefix of "highest".
            "# Exactly high",
            "> ws_1",
            "# Platform",
            "> ws_4",
            "# Mentions eb",
            "> ws_2",
            // Sections no space matches are left out.
            "---",
            "ws_1",
            "ws_2",
            "ws_3",
            "ws_4",
        ]
    );

    // Members follow the endpoint's space order when it changes.
    projected.workspaces.swap(0, 3);
    assert_eq!(
        described(&projected, &sections[..1], &HashSet::new()),
        [
            "# Any priority",
            "> ws_4",
            "> ws_3",
            "> ws_1",
            "---",
            "ws_4",
            "ws_2",
            "ws_3",
            "ws_1"
        ]
    );
}

#[test]
fn a_space_in_two_sections_is_mirrored_in_both_and_keeps_its_own_row() {
    let projected = spaces(&[
        &[("stage", "review")],
        &[("prio", "1"), ("stage", "review")],
    ]);
    let sections = pinned(
        r#"
        [[pinned]]
        title = "Priority"
        token = "prio"

        [[pinned]]
        title = "Review"
        token = "stage"
        "#,
    );

    assert_eq!(
        described(&projected, &sections, &HashSet::new()),
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
fn a_collapsed_section_keeps_its_header_and_members_but_shows_no_mirror() {
    let projected = spaces(&[&[("prio", "1")], &[("stage", "review")], &[("prio", "2")]]);
    let sections = pinned(
        r#"
        [[pinned]]
        title = "Priority"
        token = "prio"

        [[pinned]]
        title = "Review"
        token = "stage"
        "#,
    );
    let collapsed = HashSet::from(["pinned:Priority".to_owned()]);

    assert_eq!(
        described(&projected, &sections, &collapsed),
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
    let rows = sidebar_rows(&projected, &sections, &collapsed);
    let SidebarRow::Marker(header) = &rows[0] else {
        panic!("pinned header");
    };
    assert!(header.group.as_ref().is_some_and(|group| group.collapsed));
    assert_eq!(header.members, [0, 2]);
}

#[test]
fn the_title_color_is_the_configured_one() {
    let projected = spaces(&[&[("prio", "1")]]);
    let sections = pinned(
        r##"
        [[pinned]]
        title = "Long"
        token = "prio"
        fg = "#e5c07b"

        [[pinned]]
        title = "Short"
        token = "prio"
        fg = "#F80"

        [[pinned]]
        title = "Default"
        token = "prio"
        "##,
    );

    let colors = sidebar_rows(&projected, &sections, &HashSet::new())
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
        ]
    );
}

#[test]
fn pinned_sections_precede_the_groups_of_the_regular_list() {
    let projected = spaces(&[
        &[],
        &[("space_group", "work")],
        &[("space_group", "work"), ("prio", "1")],
    ]);
    let sections = pinned("[[pinned]]\ntitle = 'Priority'\ntoken = 'prio'");

    assert_eq!(
        described(&projected, &sections, &HashSet::new()),
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
