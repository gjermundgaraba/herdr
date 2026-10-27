//! Metadata tokens reported with `persist` are saved with the session.

use super::*;

#[test]
fn persisted_workspace_metadata_tokens_are_saved_with_the_session() {
    let mut app = App::new(
        &Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        tokio::sync::mpsc::unbounded_channel().1,
        crate::api::EventHub::default(),
    );
    app.state.workspaces = vec![Workspace::test_new("one")];
    app.state.ensure_test_terminals();
    let report = |persist, ttl_ms, token: &str| {
        crate::api::schema::Method::WorkspaceReportMetadata(WorkspaceReportMetadataParams {
            workspace_id: "1".into(),
            source: "plugin:test".into(),
            tokens: std::collections::HashMap::from([(token.into(), Some("on".into()))]),
            seq: None,
            ttl_ms,
            persist,
        })
    };
    for (id, method) in [
        ("persisted", report(true, None, "note")),
        ("live", report(false, None, "git")),
    ] {
        let response = app.handle_api_request(crate::api::schema::Request {
            id: id.into(),
            method,
        });
        assert!(response.contains("\"ok\""), "{response}");
    }
    let response = app.handle_api_request(crate::api::schema::Request {
        id: "expiring".into(),
        method: report(true, Some(1_000), "note"),
    });
    assert!(response.contains("invalid_metadata_ttl"), "{response}");

    let saved = crate::persist::capture(
        &app.state.workspaces,
        &app.state.terminals,
        &app.terminal_runtimes,
        Some(0),
        0,
    );
    assert_eq!(
        saved.workspaces[0].metadata_tokens,
        std::collections::HashMap::from([("note".into(), "on".into())])
    );
}
