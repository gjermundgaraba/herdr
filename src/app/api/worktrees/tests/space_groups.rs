//! Opening a worktree places its space with its family, as the fork's space
//! groups keep families together.

use super::*;

#[tokio::test]
async fn api_worktree_open_gathers_the_new_space_with_its_family() {
    let repo = create_committed_repo("api-worktree-open-gather-repo");
    let checkout = unique_temp_path("api-worktree-open-gather-checkout");
    run_git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "worktree/api-open-gather",
            checkout.to_str().unwrap(),
            "HEAD",
        ],
    );
    let event_hub = crate::api::EventHub::default();
    let mut app = test_app_with_event_hub(event_hub.clone());
    let mut parent = Workspace::test_new("main");
    parent.identity_cwd = repo.clone();
    let other = Workspace::test_new("other");
    let other_id = other.id.clone();
    app.state.workspaces = vec![parent, other];
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    app.state.selected = 0;

    let workspace_id = app.state.workspaces[0].id.clone();
    let response = run_deferred_api_request(
        &mut app,
        Request {
            id: "req".into(),
            method: crate::api::schema::Method::WorktreeOpen(WorktreeOpenParams {
                workspace_id: Some(workspace_id),
                path: Some(checkout.display().to_string()),
                ..WorktreeOpenParams::default()
            }),
        },
    );

    let success: SuccessResponse = serde_json::from_str(&response).unwrap_or_else(|err| {
        panic!("expected success response, got {response}: {err}");
    });
    let ResponseResult::WorktreeOpened { workspace, .. } = success.result else {
        panic!("expected worktree_opened response");
    };
    assert_eq!(workspace.number, 2);
    assert_eq!(app.state.workspaces[2].id, other_id);
    assert!(event_hub.events_after(0).iter().any(|(_, event)| {
        matches!(
            &event.data,
            EventData::WorkspaceReordered {
                before_workspace_id: Some(before),
                ..
            } if before == &other_id
        )
    }));

    while !app.state.workspaces.is_empty() {
        app.state.selected = 0;
        app.state.close_selected_workspace();
    }
    app.shutdown_detached_terminal_runtimes();
    let remove = crate::worktree::build_worktree_remove_command(&repo, &checkout, false, false);
    crate::worktree::run_worktree_command(&remove).unwrap();
    let _ = std::fs::remove_dir_all(repo);
}
