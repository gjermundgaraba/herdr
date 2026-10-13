//! The fork's `pane.cwd_changed` event.

use super::*;

#[test]
fn a_changed_cwd_report_emits_pane_cwd_changed_once() {
    let (mut app, _) = app_with_test_workspace();
    let pane_id = app.state.workspaces[0].tabs[0].root_pane;
    let cwd = std::env::temp_dir().join(format!("herdr-cwd-event-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    for _ in 0..2 {
        app.handle_internal_event(crate::events::AppEvent::TerminalCwdReported {
            pane_id,
            cwd: cwd.clone(),
        });
    }
    let changes = app
        .event_hub
        .events_after(0)
        .into_iter()
        .filter_map(|(_, event)| match event.data {
            EventData::PaneCwdChanged { tab_id, cwd, .. } => Some((tab_id, cwd)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        changes,
        [(app.public_tab_id(0, 0).unwrap(), cwd.display().to_string())]
    );
    let _ = std::fs::remove_dir_all(cwd);
}
