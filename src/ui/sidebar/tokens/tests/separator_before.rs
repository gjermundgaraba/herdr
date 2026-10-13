//! `separator_before` on a token table replaces the spacing before it.

use super::*;

fn separator_test_text(row: &[ResolvedToken], width: usize) -> String {
    let style = ratatui::style::Style::default();
    crate::ui::sidebar::resolved_token_spans(
        row,
        ("*", style),
        style,
        style,
        style,
        style,
        &crate::ui::sidebar::Palette::catppuccin(),
        width,
    )
    .iter()
    .map(|span| span.content.as_ref())
    .collect()
}

fn separator_space_rows(
    config: &str,
    tokens: &std::collections::HashMap<String, String>,
) -> Vec<Vec<ResolvedToken>> {
    space_rows(
        &toml::from_str(config).unwrap(),
        SpaceTokenContext {
            workspace: "repo",
            branch: Some("main"),
            state_text: "working",
            ahead_behind: Some((1, 0)),
            tokens,
            suppress_git_details: false,
        },
    )
}

#[test]
fn sidebar_separator_overrides_replace_default_spacing() {
    let tokens = std::collections::HashMap::from([("dirty".into(), "".into())]);
    let default = separator_space_rows(r#"rows = [["branch", "$dirty", "git_status"]]"#, &tokens);
    assert_eq!(separator_test_text(&default[0], 30), "main ·  ↑1");
    for (separator, expected) in [(" ", "main  ↑1"), ("", "main ↑1"), (" → ", "main →  ↑1")]
    {
        let rows = separator_space_rows(
            &format!(
                r#"rows = [["branch", {{ token = "$dirty", separator_before = "{separator}" }}, "git_status"]]"#
            ),
            &tokens,
        );
        assert_eq!(separator_test_text(&rows[0], 30), expected);
    }
    let rows = separator_space_rows(
        r#"rows = [["state_icon", { token = "branch", separator_before = "/" }, { token = "git_status", separator_before = ":" }]]"#,
        &tokens,
    );
    assert_eq!(separator_test_text(&rows[0], 30), "*/main:↑1");
}

#[test]
fn sidebar_separator_does_not_leak_from_missing_hidden_or_first_tokens() {
    let tokens = std::collections::HashMap::from([
        ("hidden".into(), "hide".into()),
        ("dirty".into(), "".into()),
    ]);
    let rows = separator_space_rows(
        r#"rows = [[{ token = "$missing", separator_before = "LEAK" }, { token = "$hidden", separator_before = "LEAK", rules = [{ equals = "hide", hide = true }] }, { token = "$dirty", separator_before = "LEADING" }, "git_status"]]"#,
        &tokens,
    );
    assert_eq!(separator_test_text(&rows[0], 30), " ↑1");
    let rows = separator_space_rows(
        r#"rows = [["branch", { token = "$hidden", separator_before = "LEAK", rules = [{ equals = "hide", hide = true }] }, { token = "$dirty", separator_before = " " }]]"#,
        &tokens,
    );
    assert_eq!(separator_test_text(&rows[0], 30), "main ");
}

#[test]
fn sidebar_separator_width_counts_when_fitting_text_tokens() {
    let tokens = std::collections::HashMap::from([("dirty".into(), "".into())]);
    let rows = separator_space_rows(
        r#"rows = [["branch", { token = "$dirty", separator_before = " " }]]"#,
        &tokens,
    );
    assert_eq!(separator_test_text(&rows[0], 6), "main ");
    assert_eq!(separator_test_text(&rows[0], 1), "");
    for separator in ["", " ", "界", " very long separator "] {
        let rows = separator_space_rows(
            &format!(
                r#"rows = [["branch", {{ token = "$dirty", separator_before = "{separator}" }}]]"#
            ),
            &tokens,
        );
        for width in 0..32 {
            let text = separator_test_text(&rows[0], width);
            assert!(
                crate::ui::sidebar::display_width(&text) <= width,
                "{text:?}, width={width}"
            );
        }
    }
}

#[test]
fn agent_sidebar_separator_override_survives_agent_specific_rows_and_rules() {
    let config: AgentsSidebarConfig = toml::from_str(r##"
rows = [["workspace", { token = "agent", separator_before = " " }]]
[rows_by_agent]
pi = [["workspace", { token = "agent", separator_before = "/", rules = [{ equals = "pi", fg = "#abc" }] }]]
"##).unwrap();
    let mut entry = entry();
    entry.canonical_agent = None;
    let rows = agent_rows(&config, context(&entry), "working");
    assert_eq!(separator_test_text(&rows[0], 30), "repo pi");
    let mut context = context(&entry);
    context.canonical_agent = Some(crate::detect::Agent::Pi);
    let rows = agent_rows(&config, context, "working");
    assert_eq!(separator_test_text(&rows[0], 30), "repo/pi");
    assert_eq!(
        rows[0][1].style.fg.unwrap().ratatui(),
        ratatui::style::Color::Rgb(0xaa, 0xbb, 0xcc)
    );
}
