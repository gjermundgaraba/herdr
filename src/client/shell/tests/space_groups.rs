use super::*;
use crate::api::schema::{Method, WorkspaceMoveBlockParams, WorkspaceMoveParams};

fn grouped_state(groups: &[Option<&str>]) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut projected = snapshot();
    let template = projected.workspaces[0].clone();
    projected.workspaces.clear();
    for (index, group) in groups.iter().enumerate() {
        let mut workspace = template.clone();
        workspace.workspace_id = format!("ws_{}", index + 1);
        workspace.number = index + 1;
        workspace.label = format!("space-{}", index + 1);
        workspace.focused = index == 0;
        workspace.tokens = group
            .map(|group| vec![("space_group".to_owned(), group.to_owned())])
            .unwrap_or_default();
        projected.workspaces.push(workspace);
    }
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state
}

fn navigation_ids(state: &ClientShellState) -> Vec<String> {
    let snapshot = state.snapshot.as_deref().expect("snapshot");
    state
        .navigation_workspace_entries(snapshot)
        .into_iter()
        .map(|entry| snapshot.workspaces[entry.index].workspace_id.clone())
        .collect()
}

fn headers(state: &ClientShellState) -> Vec<&crate::client::shell::space_groups::MarkerHit> {
    state
        .hits
        .markers
        .iter()
        .filter(|marker| marker.key.is_some())
        .collect()
}

fn header_keys(state: &ClientShellState) -> Vec<&str> {
    headers(state)
        .into_iter()
        .filter_map(|header| header.key.as_deref())
        .collect()
}

fn header_rect(state: &ClientShellState, key: &str) -> Rect {
    headers(state)
        .into_iter()
        .find(|header| header.key.as_deref() == Some(key))
        .expect("visible header")
        .rect
}

fn workspace_rect(state: &ClientShellState, workspace_id: &str) -> Rect {
    state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == workspace_id)
        .expect("visible workspace")
        .rect
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::empty(),
    })])
}

fn click(state: &mut ClientShellState, column: u16, row: u16) {
    mouse(state, MouseEventKind::Down(MouseButton::Left), column, row);
    mouse(state, MouseEventKind::Up(MouseButton::Left), column, row);
}

/// Presses at `from`, drags to `(column, row)`, releases, and returns the
/// request the drop sent, if any.
fn drag(state: &mut ClientShellState, from: Rect, column: u16, row: u16) -> Option<Method> {
    let x = from.x + 3;
    mouse(state, MouseEventKind::Down(MouseButton::Left), x, from.y);
    mouse(state, MouseEventKind::Drag(MouseButton::Left), column, row);
    let release = mouse(state, MouseEventKind::Up(MouseButton::Left), column, row);
    match &release.actions[..] {
        [ClientShellAction::Endpoint { request, .. }] => Some(request.method.clone()),
        [] => None,
        other => panic!("unexpected actions {other:?}"),
    }
}

#[test]
fn runs_render_in_server_order_with_a_divider_before_ungrouped_spaces() {
    let mut state = grouped_state(&[Some("work"), Some("work"), Some("play"), None]);
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_2", "ws_3", "ws_4"]);

    let frame = state.compose(106, 40).expect("grouped sidebar");
    let rows = frame_rows(&frame);
    assert_eq!(header_keys(&state), ["group:work", "group:play"]);
    let work = header_rect(&state, "group:work");
    assert!(rows[work.y as usize].contains("▾ work"));
    assert!(rows[work.y as usize].contains("─ 2"));
    assert_eq!(workspace_rect(&state, "ws_1").y, work.bottom());

    let play = header_rect(&state, "group:play");
    assert_eq!(play.y, workspace_rect(&state, "ws_2").bottom());

    let divider = workspace_rect(&state, "ws_4").y - 1;
    assert!(rows[divider as usize].starts_with(" ─────"));
    assert_eq!(divider, workspace_rect(&state, "ws_3").bottom());
}

#[test]
fn without_group_tokens_the_sidebar_stays_flat() {
    let mut state = grouped_state(&[None, None, None]);
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_2", "ws_3"]);
    let frame = state.compose(106, 24).expect("plain sidebar");
    assert!(state.hits.markers.is_empty());
    assert!(!frame_rows(&frame)
        .iter()
        .any(|row| row.starts_with(" ─────")));
}

#[test]
fn a_split_group_shows_a_header_for_each_run_without_reordering() {
    let mut state = grouped_state(&[Some("work"), None, Some("work")]);
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_2", "ws_3"]);
    state.compose(106, 40).expect("split group");
    assert_eq!(header_keys(&state), ["group:work", "group:work"]);
    assert_eq!(headers(&state)[0].member_ids, ["ws_1"]);
    assert_eq!(headers(&state)[1].member_ids, ["ws_3"]);
}

#[test]
fn clicking_a_header_collapses_the_group_but_keeps_its_focused_member() {
    let mut state = grouped_state(&[Some("work"), Some("work"), Some("play"), None]);
    let mut replacement = (**state.snapshot.as_ref().expect("snapshot")).clone();
    replacement.workspaces[1].agent_status = AgentStatus::Blocked;
    state.set_snapshot(Box::new(replacement));
    state.compose(106, 40).expect("expanded groups");
    let work = header_rect(&state, "group:work");

    click(&mut state, work.x + 3, work.y);
    assert!(state.collapsed_groups.contains("group:work"));
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_3", "ws_4"]);

    let frame = state.compose(106, 40).expect("collapsed group");
    let header = header_rect(&state, "group:work");
    assert!(frame_rows(&frame)[header.y as usize].contains("▸ work"));
    let (x, y) = cell_symbol_position(&frame, header, "●");
    assert_eq!(
        frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)].fg,
        crate::protocol::color_to_u32(state.config.palette.red)
    );

    let play = header_rect(&state, "group:play");
    click(&mut state, play.x + 3, play.y);
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_4"]);
}

#[test]
fn dragging_a_space_to_the_end_of_its_run_moves_it_there() {
    let mut state = grouped_state(&[Some("a"), Some("a"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let first = workspace_rect(&state, "ws_1");
    let next_header = header_rect(&state, "group:b");
    assert_eq!(
        drag(&mut state, first, first.x + 3, next_header.y - 1),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_1".into(),
            insert_index: 2,
        }))
    );
}

#[test]
fn the_first_space_of_a_later_run_moves_within_that_run() {
    let mut state = grouped_state(&[Some("a"), Some("b"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let head = workspace_rect(&state, "ws_2");
    let last = workspace_rect(&state, "ws_3");
    assert_eq!(
        drag(&mut state, head, head.x + 3, last.bottom()),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_2".into(),
            insert_index: 3,
        }))
    );
}

#[test]
fn a_space_moves_into_another_run_like_in_a_flat_sidebar() {
    let mut state = grouped_state(&[Some("a"), Some("a"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let last = workspace_rect(&state, "ws_3");
    let first_header = header_rect(&state, "group:a");
    assert_eq!(
        drag(&mut state, last, last.x + 3, first_header.y),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_3".into(),
            insert_index: 0,
        }))
    );
}

#[test]
fn a_small_jiggle_on_a_header_or_space_does_nothing() {
    let mut state = grouped_state(&[Some("a"), Some("a"), None]);
    state.compose(106, 40).expect("grouped sidebar");
    let header = header_rect(&state, "group:a");
    assert_eq!(drag(&mut state, header, header.x + 4, header.y), None);
    assert!(!state.collapsed_groups.contains("group:a"));

    state.compose(106, 40).expect("grouped sidebar");
    let space = workspace_rect(&state, "ws_2");
    assert_eq!(drag(&mut state, space, space.x + 4, space.y), None);
}

#[test]
fn dragging_a_header_moves_the_whole_run() {
    let mut state = grouped_state(&[Some("a"), Some("a"), Some("b"), None]);
    state.compose(106, 40).expect("grouped sidebar");
    let header = header_rect(&state, "group:a");
    let divider = workspace_rect(&state, "ws_4").y - 1;
    assert_eq!(
        drag(&mut state, header, header.x + 3, divider - 1),
        Some(Method::WorkspaceMoveBlock(WorkspaceMoveBlockParams {
            workspace_ids: vec!["ws_1".into(), "ws_2".into()],
            before_workspace_id: Some("ws_4".into()),
        }))
    );
    assert!(!state.collapsed_groups.contains("group:a"));
}

#[test]
fn worktree_children_follow_their_parent_group() {
    let mut state = grouped_state(&[Some("work"), None, Some("play")]);
    let mut replacement = (**state.snapshot.as_ref().expect("snapshot")).clone();
    for (index, linked) in [(0, false), (2, true)] {
        replacement.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    state.set_snapshot(Box::new(replacement));
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_3", "ws_2"]);
    state.compose(106, 40).expect("grouped worktrees");
    assert_eq!(header_keys(&state), ["group:work"]);
    assert_eq!(headers(&state)[0].member_ids, ["ws_1", "ws_3"]);
}

#[test]
fn an_overflowing_list_still_accepts_a_drop_below_its_last_visible_space() {
    let mut state = grouped_state(&[None; 12]);
    state.compose(106, 24).expect("overflowing sidebar");
    assert_eq!(state.hits.workspaces.len(), 4);
    let first = workspace_rect(&state, "ws_1");
    let last = workspace_rect(&state, "ws_4");
    assert_eq!(
        drag(&mut state, first, first.x + 3, last.bottom()),
        // Before the first hidden space, ws_5.
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_1".into(),
            insert_index: 4,
        }))
    );
}

/// Presses at `from` and drags to `(column, row)` without releasing, and
/// returns the drop target the drag shows.
fn drag_target(
    state: &mut ClientShellState,
    from: Rect,
    column: u16,
    row: u16,
) -> Option<(Option<String>, u16)> {
    mouse(
        state,
        MouseEventKind::Down(MouseButton::Left),
        from.x + 3,
        from.y,
    );
    mouse(state, MouseEventKind::Drag(MouseButton::Left), column, row);
    match &state.chrome_drag {
        Some(
            ClientChromeDrag::Workspace { target, .. }
            | ClientChromeDrag::SpaceGroup { target, .. },
        ) => target.clone(),
        _ => None,
    }
}

#[test]
fn a_header_drags_above_leading_ungrouped_spaces() {
    let mut state = grouped_state(&[None, None, Some("a")]);
    state.compose(106, 40).expect("grouped sidebar");
    let header = header_rect(&state, "group:a");
    let first = workspace_rect(&state, "ws_1");
    assert_eq!(
        drag(&mut state, header, header.x + 3, first.y - 1),
        Some(Method::WorkspaceMoveBlock(WorkspaceMoveBlockParams {
            workspace_ids: vec!["ws_3".into()],
            before_workspace_id: Some("ws_1".into()),
        }))
    );
}

#[test]
fn a_header_dropped_inside_another_group_splits_it() {
    let mut state = grouped_state(&[Some("a"), Some("b"), Some("b"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let header = header_rect(&state, "group:a");
    let middle = workspace_rect(&state, "ws_3");
    assert_eq!(
        drag(&mut state, header, header.x + 3, middle.y - 1),
        Some(Method::WorkspaceMoveBlock(WorkspaceMoveBlockParams {
            workspace_ids: vec!["ws_1".into()],
            before_workspace_id: Some("ws_3".into()),
        }))
    );
}

#[test]
fn a_collapsed_last_group_keeps_the_end_slot_below_its_header() {
    let mut state = grouped_state(&[Some("a"), Some("b"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let b = header_rect(&state, "group:b");
    click(&mut state, b.x + 3, b.y);
    state.compose(106, 40).expect("collapsed group");
    let b = header_rect(&state, "group:b");

    let space = workspace_rect(&state, "ws_1");
    assert_eq!(
        drag_target(&mut state, space, space.x + 3, b.bottom()),
        Some((None, b.bottom()))
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        space.x + 3,
        b.bottom(),
    );

    state.compose(106, 40).expect("collapsed group");
    let a = header_rect(&state, "group:a");
    assert_eq!(
        drag(&mut state, a, a.x + 3, b.bottom()),
        Some(Method::WorkspaceMoveBlock(WorkspaceMoveBlockParams {
            workspace_ids: vec!["ws_1".into()],
            before_workspace_id: None,
        }))
    );
}

#[test]
fn the_drop_line_before_a_run_sits_above_its_header() {
    let mut state = grouped_state(&[Some("a"), Some("b"), Some("b")]);
    state.compose(106, 40).expect("grouped sidebar");
    let b = header_rect(&state, "group:b");
    let space = workspace_rect(&state, "ws_1");
    assert_eq!(
        drag_target(&mut state, space, space.x + 3, b.y),
        Some((Some("ws_2".into()), b.y - 1))
    );
    let frame = state.compose(106, 40).expect("dragging");
    assert!(frame_rows(&frame)[b.y as usize].contains("▾ b"));
}

const PINNED_CONFIG: &str = r##"
[[ui.sidebar.spaces.pinned]]
title = "Priority"
token = "prio"
fg = "#e5c07b"

[[ui.sidebar.spaces.pinned]]
title = "Review"
token = "stage"
equals = "review"
"##;

fn pinned_snapshot(spaces: &[&[(&str, &str)]]) -> ClientShellSnapshot {
    let mut projected = snapshot();
    let template = projected.workspaces[0].clone();
    projected.workspaces.clear();
    for (index, tokens) in spaces.iter().enumerate() {
        let mut workspace = template.clone();
        workspace.workspace_id = format!("ws_{}", index + 1);
        workspace.number = index + 1;
        workspace.label = format!("space-{}", index + 1);
        workspace.focused = index == 0;
        workspace.tokens = tokens
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        projected.workspaces.push(workspace);
    }
    projected
}

fn pinned_state(spaces: &[&[(&str, &str)]]) -> ClientShellState {
    let config: Config = toml::from_str(PINNED_CONFIG).expect("pinned config");
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.set_snapshot(Box::new(pinned_snapshot(spaces)));
    state.set_pane_surface(surface());
    state
}

/// Server order: ws_1 is in review, ws_3 is both prioritized and in review,
/// ws_4 is prioritized, ws_2 and ws_5 match no section.
fn mixed_pinned_state() -> ClientShellState {
    pinned_state(&[
        &[("stage", "review")],
        &[],
        &[("prio", "1"), ("stage", "review")],
        &[("prio", "2")],
        &[("stage", "draft"), ("prio", "")],
    ])
}

/// Drawn space rows in order, as `(workspace_id, mirror)`.
fn drawn_spaces(state: &ClientShellState) -> Vec<(&str, bool)> {
    state
        .hits
        .workspaces
        .iter()
        .map(|hit| (hit.workspace_id.as_str(), hit.mirror))
        .collect()
}

fn real_rect(state: &ClientShellState, workspace_id: &str) -> Rect {
    state
        .hits
        .workspaces
        .iter()
        .find(|hit| !hit.mirror && hit.workspace_id == workspace_id)
        .expect("visible space")
        .rect
}

fn mirror_rect(state: &ClientShellState, workspace_id: &str) -> Rect {
    state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.mirror && hit.workspace_id == workspace_id)
        .expect("visible mirror")
        .rect
}

fn cell_fg(frame: &FrameData, x: u16, y: u16) -> u32 {
    frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)].fg
}

fn cell_bg(frame: &FrameData, x: u16, y: u16) -> u32 {
    frame.cells[usize::from(y) * usize::from(frame.width) + usize::from(x)].bg
}

#[test]
fn pinned_sections_without_matching_spaces_are_not_shown() {
    let mut state = pinned_state(&[
        &[],
        &[("prio", "")],
        &[("stage", "draft")],
        &[("other", "review")],
    ]);
    let frame = state.compose(106, 40).expect("pinned sidebar");
    assert!(state.hits.markers.is_empty());
    assert_eq!(
        drawn_spaces(&state),
        [
            ("ws_1", false),
            ("ws_2", false),
            ("ws_3", false),
            ("ws_4", false)
        ]
    );
    let rows = frame_rows(&frame);
    assert!(!rows.iter().any(|row| row.contains("Priority")));
    assert!(!rows.iter().any(|row| row.starts_with(" ─────")));
}

#[test]
fn pinned_sections_mirror_matching_spaces_in_config_order_above_the_list() {
    let mut state = mixed_pinned_state();
    let frame = state.compose(106, 60).expect("pinned sidebar");
    let rows = frame_rows(&frame);

    assert_eq!(
        drawn_spaces(&state),
        [
            ("ws_3", true),
            ("ws_4", true),
            ("ws_1", true),
            ("ws_3", true),
            ("ws_1", false),
            ("ws_2", false),
            ("ws_3", false),
            ("ws_4", false),
            ("ws_5", false),
        ]
    );
    assert_eq!(
        header_keys(&state),
        ["pinned:Priority", "pinned:Review"],
        "pinned headers come first, in config order"
    );
    assert!(state
        .hits
        .markers
        .iter()
        .all(|marker| marker.key.is_none() != marker.pinned));

    let priority = header_rect(&state, "pinned:Priority");
    assert_eq!(priority.y, state.hits.workspace_body.y);
    assert!(rows[priority.y as usize].contains("▾ Priority"));
    assert!(rows[priority.y as usize].contains("─ 2"));
    assert_eq!(mirror_rect(&state, "ws_4").y, priority.bottom() + 2);
    let (x, y) = cell_symbol_position(&frame, priority, "Priority");
    assert_eq!(
        cell_fg(&frame, x, y),
        crate::protocol::color_to_u32(ratatui::style::Color::Rgb(0xe5, 0xc0, 0x7b))
    );

    let review = header_rect(&state, "pinned:Review");
    assert_eq!(review.y, mirror_rect(&state, "ws_4").bottom());
    assert!(rows[review.y as usize].contains("▾ Review"));
    assert!(rows[review.y as usize].contains("─ 2"));
    let (x, y) = cell_symbol_position(&frame, review, "Review");
    assert_eq!(
        cell_fg(&frame, x, y),
        crate::protocol::color_to_u32(state.config.palette.subtext0)
    );

    // The regular list follows behind a divider, unchanged.
    let divider = real_rect(&state, "ws_1").y - 1;
    assert!(rows[divider as usize].starts_with(" ─────"));
    let divider_hit = state
        .hits
        .markers
        .iter()
        .find(|marker| marker.rect.y == divider)
        .expect("divider hit");
    assert!(!divider_hit.pinned && divider_hit.key.is_none());
    assert_eq!(
        navigation_ids(&state),
        ["ws_1", "ws_2", "ws_3", "ws_4", "ws_5"]
    );
}

#[test]
fn pinned_sections_precede_groups_and_keep_group_headers() {
    let mut state = pinned_state(&[
        &[("space_group", "work"), ("prio", "1")],
        &[("space_group", "work")],
        &[],
    ]);
    state.compose(106, 60).expect("pinned sidebar");
    assert_eq!(
        header_keys(&state),
        ["pinned:Priority", "group:work"],
        "a grouped first run starts with its own header, not a divider"
    );
    assert_eq!(
        header_rect(&state, "group:work").y,
        mirror_rect(&state, "ws_1").bottom()
    );
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_2", "ws_3"]);
}

#[test]
fn linked_worktree_mirrors_render_flat() {
    let mut projected = pinned_snapshot(&[&[], &[], &[("prio", "1")]]);
    for (index, linked) in [(0, false), (2, true)] {
        projected.workspaces[index].worktree = Some(ClientShellWorktree {
            key: "repo".into(),
            label: "repo".into(),
            is_linked_worktree: linked,
        });
    }
    let mut state = pinned_state(&[]);
    state.set_snapshot(Box::new(projected));
    let frame = state.compose(106, 60).expect("pinned worktrees");
    let mirror = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.mirror)
        .expect("mirror");
    assert_eq!(mirror.workspace_id, "ws_3");
    assert!(!mirror.indented);
    assert!(mirror.group_toggle.is_none());
    let real = state
        .hits
        .workspaces
        .iter()
        .find(|hit| !hit.mirror && hit.workspace_id == "ws_3")
        .expect("real row");
    assert!(real.indented);
    assert!(!frame_rows(&frame)[mirror.rect.y as usize].contains("└─"));
    assert_eq!(navigation_ids(&state), ["ws_1", "ws_3", "ws_2"]);
}

#[test]
fn clicking_a_pinned_header_collapses_only_that_section() {
    let mut state = mixed_pinned_state();
    let mut replacement = (**state.snapshot.as_ref().expect("snapshot")).clone();
    replacement.workspaces[3].agent_status = AgentStatus::Blocked;
    state.set_snapshot(Box::new(replacement));
    state.compose(106, 60).expect("pinned sidebar");
    let priority = header_rect(&state, "pinned:Priority");

    click(&mut state, priority.x + 3, priority.y);
    assert!(state.collapsed_groups.contains("pinned:Priority"));
    assert!(!state.collapsed_groups.contains("group:Priority"));
    assert_eq!(
        navigation_ids(&state),
        ["ws_1", "ws_2", "ws_3", "ws_4", "ws_5"]
    );

    let frame = state.compose(106, 60).expect("collapsed section");
    let priority = header_rect(&state, "pinned:Priority");
    assert!(frame_rows(&frame)[priority.y as usize].contains("▸ Priority"));
    let (x, y) = cell_symbol_position(&frame, priority, "●");
    assert_eq!(
        cell_fg(&frame, x, y),
        crate::protocol::color_to_u32(state.config.palette.red)
    );
    // Even the focused space's mirror hides; the Review section is untouched.
    assert_eq!(
        drawn_spaces(&state)
            .into_iter()
            .filter(|(_, mirror)| *mirror)
            .collect::<Vec<_>>(),
        [("ws_1", true), ("ws_3", true)]
    );
    assert_eq!(
        header_rect(&state, "pinned:Review").y,
        priority.bottom(),
        "a collapsed header stays attached to the next one"
    );

    click(&mut state, priority.x + 3, priority.y);
    assert!(!state.collapsed_groups.contains("pinned:Priority"));
    state.compose(106, 60).expect("expanded section");
    assert_eq!(mirror_rect(&state, "ws_4").y, priority.bottom() + 2);
}

#[test]
fn a_group_and_a_pinned_section_with_one_name_collapse_separately() {
    let mut state = pinned_state(&[&[("space_group", "Priority"), ("prio", "1")], &[]]);
    state.compose(106, 60).expect("pinned sidebar");
    let group = header_rect(&state, "group:Priority");
    click(&mut state, group.x + 3, group.y);
    assert!(state.collapsed_groups.contains("group:Priority"));
    state.compose(106, 60).expect("collapsed group");
    assert_eq!(header_keys(&state), ["pinned:Priority", "group:Priority"]);
    mirror_rect(&state, "ws_1");
}

#[test]
fn keyboard_navigation_skips_mirrors_and_mirrors_show_the_selection() {
    let mut state = mixed_pinned_state();
    state.compose(106, 60).expect("pinned sidebar");
    state.handle_input_bytes(&[0x02]);
    state.handle_input_bytes(b"w");
    assert_eq!(state.mode, ClientShellMode::Navigate);

    let mut visited = Vec::new();
    for _ in 0..6 {
        let selected = state
            .navigate_workspace_id
            .as_ref()
            .expect("navigation selection")
            .workspace_id
            .clone();
        visited.push(selected);
        state.handle_input_bytes(b"\x1b[B");
    }
    assert_eq!(visited, ["ws_1", "ws_2", "ws_3", "ws_4", "ws_5", "ws_1"]);

    // Now on ws_2; one more step selects ws_3, which has two mirrors.
    state.handle_input_bytes(b"\x1b[B");
    let frame = state.compose(106, 60).expect("navigating");
    let palette = &state.config.palette;
    let selection = if palette.selection_bg == ratatui::style::Color::Reset {
        palette.active_row_bg
    } else {
        palette.selection_bg
    };
    let selection = crate::protocol::color_to_u32(selection);
    let rows = state
        .hits
        .workspaces
        .iter()
        .filter(|hit| hit.workspace_id == "ws_3")
        .map(|hit| hit.rect)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 3);
    for rect in rows {
        assert_eq!(cell_bg(&frame, rect.x + 2, rect.y), selection, "{rect:?}");
    }
    let other = mirror_rect(&state, "ws_4");
    assert_ne!(cell_bg(&frame, other.x + 2, other.y), selection);
}

#[test]
fn mirrors_of_the_focused_space_use_the_focused_style() {
    let mut state = mixed_pinned_state();
    let frame = state.compose(106, 60).expect("pinned sidebar");
    let active = crate::protocol::color_to_u32(state.config.palette.active_row_bg);
    let real = real_rect(&state, "ws_1");
    let mirror = mirror_rect(&state, "ws_1");
    assert_eq!(cell_bg(&frame, real.x + 2, real.y), active);
    assert_eq!(cell_bg(&frame, mirror.x + 2, mirror.y), active);
    let other = mirror_rect(&state, "ws_4");
    assert_ne!(cell_bg(&frame, other.x + 2, other.y), active);
}

#[test]
fn clicking_a_mirror_focuses_the_real_space() {
    let mut state = mixed_pinned_state();
    state.compose(106, 60).expect("pinned sidebar");
    let mirror = mirror_rect(&state, "ws_4");
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        mirror.x + 3,
        mirror.y,
    );
    let release = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        mirror.x + 3,
        mirror.y,
    );
    let [ClientShellAction::Endpoint { request, .. }] = &release.actions[..] else {
        panic!("mirror click should focus through the endpoint API");
    };
    assert!(matches!(
        &request.method,
        Method::WorkspaceFocus(target) if target.workspace_id == "ws_4"
    ));
}

#[test]
fn right_clicking_a_mirror_opens_the_space_menu() {
    let mut state = mixed_pinned_state();
    state.compose(106, 60).expect("pinned sidebar");
    let mirror = mirror_rect(&state, "ws_3");
    let open = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Right),
        mirror.x + 3,
        mirror.y,
    );
    assert!(open.actions.is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ContextMenu(ClientContextMenuOverlay {
            target: ClientContextMenuTarget::Workspace { ref workspace_id, .. },
            ..
        })) if workspace_id == "ws_3"
    ));
}

#[test]
fn mirrors_and_pinned_headers_do_not_drag() {
    let mut state = mixed_pinned_state();
    state.compose(106, 60).expect("pinned sidebar");
    let mirror = mirror_rect(&state, "ws_4");
    let last = real_rect(&state, "ws_5");
    assert_eq!(
        drag_target(&mut state, mirror, mirror.x + 3, last.bottom()),
        None
    );
    assert!(state.chrome_drag.is_none());
    // Releasing after the pointer moved is a plain click, never a move.
    let release = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        mirror.x + 3,
        last.bottom(),
    );
    assert!(matches!(
        &release.actions[..],
        [ClientShellAction::Endpoint { request, .. }]
            if matches!(&request.method, Method::WorkspaceFocus(target) if target.workspace_id == "ws_4")
    ));

    state.compose(106, 60).expect("pinned sidebar");
    let header = header_rect(&state, "pinned:Review");
    assert_eq!(
        drag_target(&mut state, header, header.x + 3, last.bottom()),
        None
    );
    assert!(state.chrome_drag.is_none());
}

#[test]
fn pinned_sections_take_no_drops_but_the_regular_list_still_does() {
    let mut state = mixed_pinned_state();
    state.compose(106, 60).expect("pinned sidebar");
    let space = real_rect(&state, "ws_4");
    let mirror = mirror_rect(&state, "ws_1");
    let header = header_rect(&state, "pinned:Priority");
    let divider = real_rect(&state, "ws_1").y - 1;
    for row in [header.y, mirror.y, mirror.bottom() - 1] {
        // Start a drag in the regular list, then move over the pinned rows.
        assert!(drag_target(&mut state, space, space.x + 3, divider).is_some());
        mouse(
            &mut state,
            MouseEventKind::Drag(MouseButton::Left),
            space.x + 3,
            row,
        );
        assert!(matches!(
            state.chrome_drag,
            Some(ClientChromeDrag::Workspace { target: None, .. })
        ));
        let release = mouse(
            &mut state,
            MouseEventKind::Up(MouseButton::Left),
            space.x + 3,
            row,
        );
        assert!(release.actions.is_empty(), "{row}");
        state.compose(106, 60).expect("pinned sidebar");
    }

    // The divider row stands for the first regular space.
    assert_eq!(
        drag(&mut state, space, space.x + 3, divider),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_4".into(),
            insert_index: 0,
        }))
    );

    state.compose(106, 60).expect("pinned sidebar");
    let first = real_rect(&state, "ws_1");
    let last = real_rect(&state, "ws_5");
    assert_eq!(
        drag(&mut state, first, first.x + 3, last.bottom()),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_1".into(),
            insert_index: 5,
        }))
    );

    state.compose(106, 60).expect("pinned sidebar");
    let third = real_rect(&state, "ws_3");
    assert_eq!(
        drag(&mut state, first, first.x + 3, third.y - 1),
        Some(Method::WorkspaceMove(WorkspaceMoveParams {
            workspace_id: "ws_1".into(),
            insert_index: 2,
        }))
    );
}

#[test]
fn an_overflowing_pinned_sidebar_scrolls_to_and_reveals_real_rows() {
    let spaces: Vec<&[(&str, &str)]> = vec![&[("prio", "1")]; 8];
    let mut state = pinned_state(&spaces);
    // The first frame reveals the focused space's real row; start at the top.
    state.compose(106, 24).expect("overflowing sidebar");
    state.workspace_scroll = 0;
    state.compose(106, 24).expect("overflowing sidebar");
    assert_eq!(
        header_keys(&state),
        ["pinned:Priority"],
        "the list starts at the pinned header"
    );
    assert!(!drawn_spaces(&state).contains(&("ws_8", false)));

    state.reveal_workspace("ws_8");
    state.compose(106, 24).expect("revealed space");
    assert!(drawn_spaces(&state).contains(&("ws_8", false)));

    // Header, eight mirrors, divider, and eight spaces: scrolling ends on
    // the last real row.
    state.workspace_scroll = usize::MAX;
    state.compose(106, 24).expect("scrolled to the end");
    assert_eq!(drawn_spaces(&state).last(), Some(&("ws_8", false)));
    let metrics = state.hits.workspace_scroll_metrics.expect("metrics");
    assert!(metrics.max_offset_from_bottom > 9);
}

#[test]
fn each_machine_shows_its_own_pinned_sections() {
    use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};

    let mut state = pinned_state(&[&[("prio", "1")], &[]]);
    let profile = SavedSshEndpoint {
        id: ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    let mut remote_snapshot = pinned_snapshot(&[&[], &[("stage", "review")]]);
    remote_snapshot.boot_id = "remote-boot".into();
    state.set_endpoint_snapshot(&remote, Box::new(remote_snapshot));

    let frame = state.compose(120, 60).expect("machines sidebar");
    let drawn = state
        .hits
        .workspaces
        .iter()
        .map(|hit| {
            (
                hit.endpoint_id.is_local(),
                hit.workspace_id.as_str(),
                hit.mirror,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        drawn,
        [
            (true, "ws_1", true),
            (true, "ws_1", false),
            (true, "ws_2", false),
            (false, "ws_2", true),
            (false, "ws_1", false),
            (false, "ws_2", false),
        ]
    );
    let pinned = state
        .hits
        .markers
        .iter()
        .filter(|marker| marker.pinned)
        .map(|marker| (marker.endpoint_id.is_local(), marker.key.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        pinned,
        [
            (true, Some("pinned:Priority")),
            (false, Some("pinned:Review"))
        ]
    );
    let header = state
        .hits
        .markers
        .iter()
        .find(|marker| !marker.endpoint_id.is_local() && marker.pinned)
        .expect("remote pinned header")
        .rect;
    assert!(frame_rows(&frame)[header.y as usize].contains("▾ Review"));

    click(&mut state, header.x + 3, header.y);
    assert!(state.remote_collapsed_groups[&remote].contains("pinned:Review"));
    assert!(!state.collapsed_groups.contains("pinned:Review"));
    state.compose(120, 60).expect("collapsed remote section");
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| !hit.endpoint_id.is_local() && hit.mirror));
    assert!(state
        .hits
        .workspaces
        .iter()
        .any(|hit| hit.endpoint_id.is_local() && hit.mirror));
}
