//! The `pinned_sections` the fork's snapshot resolves from the config.

use super::priority_keys::{app_with_agents, set_token};
use super::*;

fn pin(app: &mut crate::app::App, sections: &str) {
    app.state.sidebar_spaces =
        toml::from_str::<crate::config::SpacesSidebarConfig>(sections).unwrap();
}

#[test]
fn snapshot_resolves_pinned_sections_in_config_and_space_order() {
    use crate::detect::AgentState::Idle;
    let mut app = app_with_agents(&[
        ("api", Idle, true),
        ("web", Idle, true),
        ("docs", Idle, true),
        ("infra", Idle, true),
    ]);
    set_token(&mut app, 0, "prio", "high");
    set_token(&mut app, 1, "prio", "");
    set_token(&mut app, 2, "prio", "low");
    set_token(&mut app, 3, "prio", "highest");
    set_token(&mut app, 3, "team", "platform-eu");
    set_token(&mut app, 1, "team", "web");
    pin(
        &mut app,
        r##"
        [[pinned]]
        title = "Any priority"
        token = "prio"
        fg = "#F80"

        [[pinned]]
        title = "Exactly high"
        token = "prio"
        equals = "high"

        [[pinned]]
        title = "Platform"
        token = "team"
        starts_with = "platform"
        fg = "#0a1b2c"

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

    let first = snapshot(&app, "boot", 1, None, None);

    let id = |index: usize| first.workspaces[index].workspace_id.clone();
    let section =
        |title: &str, fg: Option<&str>, members: &[usize]| protocol::ClientShellPinnedSection {
            title: title.into(),
            fg: fg.map(str::to_owned),
            workspace_ids: members.iter().map(|&index| id(index)).collect(),
        };
    assert_eq!(
        first.pinned_sections,
        [
            // An empty value pins nothing; members keep the space order.
            section("Any priority", Some("#ff8800"), &[0, 2, 3]),
            // `equals` is the whole value, not a prefix of "highest".
            section("Exactly high", None, &[0]),
            section("Platform", Some("#0a1b2c"), &[3]),
            section("Mentions eb", None, &[1]),
            // Sections no space matches are left out.
        ]
    );

    // Members follow the endpoint's space order when it changes.
    app.state.workspaces.swap(0, 3);
    let reordered = snapshot(&app, "boot", 2, None, None);
    assert_eq!(
        reordered.pinned_sections[0].workspace_ids,
        [id(3), id(2), id(0)]
    );
    assert_eq!(
        reordered
            .workspaces
            .iter()
            .map(|workspace| workspace.workspace_id.clone())
            .collect::<Vec<_>>(),
        [id(3), id(1), id(2), id(0)]
    );

    pin(&mut app, "");
    assert!(snapshot(&app, "boot", 3, None, None)
        .pinned_sections
        .is_empty());
}
