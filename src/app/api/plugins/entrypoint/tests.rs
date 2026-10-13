fn app_with_unfocused_target() -> crate::app::App {
    let mut app = super::super::tests::test_app();
    let mut target = crate::workspace::Workspace::test_new("target");
    target.custom_name = Some("Target".into());
    target.test_add_tab(Some("second"));
    app.state.workspaces = vec![crate::workspace::Workspace::test_new("focused"), target];
    app.state.ensure_test_terminals();
    app.state.active = Some(0);
    app
}

#[test]
fn targets_supply_context_most_specific_first() {
    let app = app_with_unfocused_target();
    let target_tab = app.public_tab_id(1, 1).unwrap();
    let target_pane = app
        .public_pane_id(1, app.state.workspaces[1].tabs[1].root_pane)
        .unwrap();
    let workspace_id = app.public_workspace_id(1);
    for (tab_id, pane_id) in [
        (None, Some(target_pane.as_str())),
        (Some(target_tab.as_str()), Some(target_pane.as_str())),
        (Some(target_tab.as_str()), None),
    ] {
        let context = app
            .plugin_context_for_target(Some(&workspace_id), tab_id, pane_id, "menu")
            .unwrap();
        assert_eq!(context.workspace_id.as_deref(), Some(workspace_id.as_str()));
        assert_eq!(context.workspace_label.as_deref(), Some("Target"));
        assert_eq!(context.tab_id.as_deref(), Some(target_tab.as_str()));
        assert_eq!(
            context.focused_pane_id.as_deref(),
            Some(target_pane.as_str())
        );
    }
}

#[test]
fn targets_reject_unknown_and_disagreeing_ids() {
    let app = app_with_unfocused_target();
    let target_pane = app
        .public_pane_id(1, app.state.workspaces[1].tabs[1].root_pane)
        .unwrap();
    let focused_workspace = app.public_workspace_id(0);
    for (workspace_id, pane_id, code) in [
        (None, "missing", "pane_not_found"),
        (
            Some(focused_workspace.as_str()),
            target_pane.as_str(),
            "target_mismatch",
        ),
    ] {
        let error = app
            .plugin_context_for_target(workspace_id, None, Some(pane_id), "menu")
            .unwrap_err();
        assert_eq!(error.code, code);
    }
}
