//! A live reload with invalid `ui.agent_priority_tokens` keeps the current UI.

use super::*;

#[test]
fn load_live_config_keeps_current_ui_on_invalid_agent_priority_tokens() {
    let loaded = load_live_config_from_str(
        r#"
[ui]
agent_priority_tokens = ["lift", "lift"]
"#,
    )
    .unwrap();

    assert_eq!(loaded.invalid_sections, vec!["ui"]);
    assert_eq!(loaded.diagnostics.len(), 1);
    assert!(loaded.diagnostics[0].contains("duplicate ui.agent_priority_tokens key"));
    assert!(loaded.diagnostics[0].contains("keeping current ui settings"));
}
