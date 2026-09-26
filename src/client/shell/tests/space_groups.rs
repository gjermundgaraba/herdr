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
