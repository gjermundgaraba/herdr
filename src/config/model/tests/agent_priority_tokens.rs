//! `ui.agent_priority_tokens` parsing and validation.

use super::*;

#[test]
fn agent_priority_tokens_parse_in_order_and_default_empty() {
    assert!(Config::default().ui.agent_priority_tokens.is_empty());

    let toml = r#"
[ui]
agent_priority_tokens = ["lift", "urgent-2"]
"#;
    let config: Config = toml::from_str(toml).unwrap();
    assert_eq!(config.ui.agent_priority_tokens, ["lift", "urgent-2"]);
}

#[test]
fn agent_priority_tokens_reject_invalid_duplicate_and_excess_keys() {
    let parse = |tokens: &str| {
        toml::from_str::<Config>(&format!("[ui]\nagent_priority_tokens = {tokens}\n"))
            .unwrap_err()
            .to_string()
    };
    assert!(parse(r#"[""]"#).contains("invalid ui.agent_priority_tokens key \"\""));
    assert!(parse(r#"["bad.key"]"#).contains("invalid ui.agent_priority_tokens key"));
    assert!(parse(&format!("[\"{}\"]", "x".repeat(33))).contains("invalid"));
    assert!(parse(r#"["lift", "lift"]"#).contains("duplicate ui.agent_priority_tokens key"));
    let many = (0..=MAX_AGENT_PRIORITY_TOKENS)
        .map(|index| format!("\"t{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(parse(&format!("[{many}]")).contains("at most 16 tokens"));
    assert!(toml::from_str::<Config>("[ui]\nagent_priority_tokens = []\n").is_ok());
}
