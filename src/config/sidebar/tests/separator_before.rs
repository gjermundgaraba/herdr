use super::*;

#[test]
fn sidebar_separator_overrides_round_trip_and_drop_control_characters() {
    let input = r##"
[agents]
rows = [["workspace", { token = "$summary", separator_before = "", fg = "#abc" }]]
[agents.rows_by_agent]
pi = [[{ token = "agent", separator_before = " / " }]]
[spaces]
rows = [["branch", { token = "$git_dirty", separator_before = "\t |\u001b\n" }]]
"##;
    let config: SidebarConfig = toml::from_str(input).unwrap();
    assert_eq!(config.agents.rows[0][0].separator_before(), None);
    assert_eq!(
        config.agents.rows[0][1].separator_before().as_deref(),
        Some("")
    );
    assert_eq!(
        config.agents.rows_by_agent["pi"][0][0]
            .separator_before()
            .as_deref(),
        Some(" / ")
    );
    assert_eq!(
        config.spaces.rows[0][1].separator_before().as_deref(),
        Some(" |")
    );
    let encoded = toml::to_string(&config).unwrap();
    assert_eq!(toml::from_str::<SidebarConfig>(&encoded).unwrap(), config);
    let unseparated: SidebarConfig =
        toml::from_str(r##"agents.rows = [[{ token = "agent", fg = "#abc" }]]"##).unwrap();
    assert!(!toml::to_string(&unseparated)
        .unwrap()
        .contains("separator_before"));
}
