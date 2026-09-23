mod rules;

pub use rules::SidebarTokenRule;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::detect::Agent;

const MAX_SIDEBAR_ROWS: usize = 16;
const MAX_SIDEBAR_TOKENS_PER_ROW: usize = 16;
const DEFAULT_SIDEBAR_ROW_GAP: u16 = 0;

fn deserialize_sidebar_rows<'de, D, T>(deserializer: D) -> Result<Vec<Vec<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let rows = Vec::<Vec<T>>::deserialize(deserializer)?;
    validate_sidebar_rows(&rows).map_err(serde::de::Error::custom)?;
    Ok(rows)
}

fn validate_sidebar_rows<T>(rows: &[Vec<T>]) -> Result<(), String> {
    if rows.len() > MAX_SIDEBAR_ROWS {
        return Err(format!(
            "sidebar layouts may contain at most {MAX_SIDEBAR_ROWS} rows"
        ));
    }
    if rows
        .iter()
        .any(|row| row.len() > MAX_SIDEBAR_TOKENS_PER_ROW)
    {
        return Err(format!(
            "sidebar rows may contain at most {MAX_SIDEBAR_TOKENS_PER_ROW} tokens"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarTokenColor {
    r: u8,
    g: u8,
    b: u8,
}

impl SidebarTokenColor {
    pub(crate) fn ratatui(self) -> ratatui::style::Color {
        ratatui::style::Color::Rgb(self.r, self.g, self.b)
    }
}

impl Serialize for SidebarTokenColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b))
    }
}

impl<'de> Deserialize<'de> for SidebarTokenColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let hex = value.strip_prefix('#').filter(|hex| {
            hex.is_ascii()
                && matches!(hex.len(), 3 | 6)
                && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let Some(hex) = hex else {
            return Err(serde::de::Error::custom(
                "sidebar token fg must be #RGB or #RRGGBB",
            ));
        };
        let (r, g, b) = if hex.len() == 3 {
            let mut digits = hex
                .bytes()
                .map(|byte| char::from(byte).to_digit(16).expect("validated hex digit") as u8 * 17);
            (
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
                digits.next().expect("three hex digits"),
            )
        } else {
            (
                u8::from_str_radix(&hex[0..2], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[2..4], 16).expect("validated hex digits"),
                u8::from_str_radix(&hex[4..6], 16).expect("validated hex digits"),
            )
        };
        Ok(Self { r, g, b })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarTokenStyle {
    pub fg: Option<SidebarTokenColor>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSidebarToken {
    StateIcon,
    StateText,
    Machine,
    Workspace,
    Tab,
    Pane,
    Agent,
    TerminalTitle,
    TerminalTitleStripped,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceSidebarToken {
    StateIcon,
    StateText,
    Workspace,
    Branch,
    GitStatus,
    Custom(String),
}

/// A token kind that can appear in a sidebar row, named as it is in config.
pub trait SidebarTokenKind: Sized {
    fn name(&self) -> String;
    fn parse(value: String) -> Result<Self, String>;
}

/// One occurrence of a token in a sidebar row, with its per-occurrence options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarTokenSpec<T> {
    pub token: T,
    pub style: SidebarTokenStyle,
    pub rules: Vec<SidebarTokenRule>,
    pub separator_before: Option<Arc<str>>,
}

impl<T> SidebarTokenSpec<T> {
    pub(crate) fn style_for_value(&self, value: &str) -> Option<SidebarTokenStyle> {
        rules::matching_style(&self.rules, self.style, value)
    }
}

impl<T> From<T> for SidebarTokenSpec<T> {
    fn from(token: T) -> Self {
        Self {
            token,
            style: SidebarTokenStyle::default(),
            rules: Vec::new(),
            separator_before: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStyledSidebarToken {
    token: String,
    #[serde(default)]
    separator_before: Option<String>,
    #[serde(default)]
    fg: Option<SidebarTokenColor>,
    #[serde(default)]
    bold: Option<bool>,
    #[serde(default)]
    dim: Option<bool>,
    #[serde(default)]
    rules: Vec<SidebarTokenRule>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawSidebarToken {
    Plain(String),
    Styled(RawStyledSidebarToken),
}

impl<'de, T: SidebarTokenKind> Deserialize<'de> for SidebarTokenSpec<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = match RawSidebarToken::deserialize(deserializer)? {
            RawSidebarToken::Plain(token) => {
                return T::parse(token)
                    .map(Self::from)
                    .map_err(serde::de::Error::custom)
            }
            RawSidebarToken::Styled(raw) => raw,
        };
        if raw.rules.len() > 16 {
            return Err(serde::de::Error::custom(
                "sidebar tokens may contain at most 16 rules",
            ));
        }
        if !raw.rules.is_empty() && matches!(raw.token.as_str(), "state_icon" | "git_status") {
            return Err(serde::de::Error::custom(
                "sidebar rules require a text-valued token",
            ));
        }
        Ok(Self {
            token: T::parse(raw.token).map_err(serde::de::Error::custom)?,
            style: SidebarTokenStyle {
                fg: raw.fg,
                bold: raw.bold,
                dim: raw.dim,
            },
            rules: raw.rules,
            separator_before: raw.separator_before.map(|separator| {
                separator
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .collect::<String>()
                    .into()
            }),
        })
    }
}

impl<T: SidebarTokenKind> Serialize for SidebarTokenSpec<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let name = self.token.name();
        if self.style == SidebarTokenStyle::default()
            && self.rules.is_empty()
            && self.separator_before.is_none()
        {
            return serializer.serialize_str(&name);
        }
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("token", &name)?;
        if let Some(separator) = &self.separator_before {
            map.serialize_entry("separator_before", &separator[..])?;
        }
        if let Some(fg) = self.style.fg {
            map.serialize_entry("fg", &fg)?;
        }
        if let Some(bold) = self.style.bold {
            map.serialize_entry("bold", &bold)?;
        }
        if let Some(dim) = self.style.dim {
            map.serialize_entry("dim", &dim)?;
        }
        if !self.rules.is_empty() {
            map.serialize_entry("rules", &self.rules)?;
        }
        map.end()
    }
}

fn parse_sidebar_token<T>(value: String, builtins: &[(&str, T)]) -> Result<T, String>
where
    T: Clone + From<String>,
{
    if let Some((_, token)) = builtins.iter().find(|(name, _)| *name == value) {
        return Ok(token.clone());
    }
    let Some(name) = value.strip_prefix('$') else {
        return Err(format!(
            "unknown sidebar token `{value}`; custom tokens must start with `$`"
        ));
    };
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(format!("invalid custom sidebar token `{value}`"));
    }
    Ok(T::from(name.to_string()))
}

impl SidebarTokenKind for AgentSidebarToken {
    fn name(&self) -> String {
        match self {
            Self::StateIcon => "state_icon".into(),
            Self::StateText => "state_text".into(),
            Self::Machine => "machine".into(),
            Self::Workspace => "workspace".into(),
            Self::Tab => "tab".into(),
            Self::Pane => "pane".into(),
            Self::Agent => "agent".into(),
            Self::TerminalTitle => "terminal_title".into(),
            Self::TerminalTitleStripped => "terminal_title_stripped".into(),
            Self::Custom(name) => format!("${name}"),
        }
    }

    fn parse(value: String) -> Result<Self, String> {
        parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("machine", Self::Machine),
                ("workspace", Self::Workspace),
                ("tab", Self::Tab),
                ("pane", Self::Pane),
                ("agent", Self::Agent),
                ("terminal_title", Self::TerminalTitle),
                ("terminal_title_stripped", Self::TerminalTitleStripped),
            ],
        )
    }
}

impl From<String> for AgentSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

impl SidebarTokenKind for SpaceSidebarToken {
    fn name(&self) -> String {
        match self {
            Self::StateIcon => "state_icon".into(),
            Self::StateText => "state_text".into(),
            Self::Workspace => "workspace".into(),
            Self::Branch => "branch".into(),
            Self::GitStatus => "git_status".into(),
            Self::Custom(name) => format!("${name}"),
        }
    }

    fn parse(value: String) -> Result<Self, String> {
        parse_sidebar_token(
            value,
            &[
                ("state_icon", Self::StateIcon),
                ("state_text", Self::StateText),
                ("workspace", Self::Workspace),
                ("branch", Self::Branch),
                ("git_status", Self::GitStatus),
            ],
        )
    }
}

impl From<String> for SpaceSidebarToken {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

type AgentSidebarRows = Vec<Vec<SidebarTokenSpec<AgentSidebarToken>>>;
type SpaceSidebarRows = Vec<Vec<SidebarTokenSpec<SpaceSidebarToken>>>;

fn deserialize_rows_by_agent<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, AgentSidebarRows>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let rows_by_agent = BTreeMap::<String, AgentSidebarRows>::deserialize(deserializer)?;
    for (id, rows) in &rows_by_agent {
        if crate::detect::parse_canonical_agent_label(id).is_none() {
            return Err(serde::de::Error::custom(format!(
                "unknown canonical agent id `{id}` in sidebar rows_by_agent"
            )));
        }
        validate_sidebar_rows(rows).map_err(serde::de::Error::custom)?;
    }
    Ok(rows_by_agent)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentsSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: AgentSidebarRows,
    #[serde(default, deserialize_with = "deserialize_rows_by_agent")]
    pub rows_by_agent: BTreeMap<String, AgentSidebarRows>,
    pub row_gap: u16,
}

impl AgentsSidebarConfig {
    pub(crate) fn rows_for_agent(&self, agent: Option<Agent>) -> &AgentSidebarRows {
        agent
            .and_then(|agent| self.rows_by_agent.get(crate::detect::agent_label(agent)))
            .unwrap_or(&self.rows)
    }
}

impl Default for AgentsSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![
                    AgentSidebarToken::StateIcon.into(),
                    AgentSidebarToken::Machine.into(),
                    AgentSidebarToken::Workspace.into(),
                    AgentSidebarToken::Tab.into(),
                ],
                vec![AgentSidebarToken::Agent.into()],
            ],
            rows_by_agent: BTreeMap::new(),
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SpacesSidebarConfig {
    #[serde(deserialize_with = "deserialize_sidebar_rows")]
    pub rows: SpaceSidebarRows,
    pub row_gap: u16,
}

impl Default for SpacesSidebarConfig {
    fn default() -> Self {
        Self {
            rows: vec![
                vec![
                    SpaceSidebarToken::StateIcon.into(),
                    SpaceSidebarToken::Workspace.into(),
                ],
                vec![
                    SpaceSidebarToken::Branch.into(),
                    SpaceSidebarToken::GitStatus.into(),
                ],
            ],
            row_gap: DEFAULT_SIDEBAR_ROW_GAP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SidebarConfig {
    pub agents: AgentsSidebarConfig,
    pub spaces: SpacesSidebarConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_compact_agent_and_existing_space_layouts() {
        let config = SidebarConfig::default();
        assert_eq!(
            config.agents.rows,
            vec![
                vec![
                    AgentSidebarToken::StateIcon.into(),
                    AgentSidebarToken::Machine.into(),
                    AgentSidebarToken::Workspace.into(),
                    AgentSidebarToken::Tab.into(),
                ],
                vec![AgentSidebarToken::Agent.into()],
            ]
        );
        assert!(config.agents.rows_by_agent.is_empty());
        assert_eq!(config.agents.row_gap, 0);
        assert_eq!(
            config.spaces.rows,
            vec![
                vec![
                    SpaceSidebarToken::StateIcon.into(),
                    SpaceSidebarToken::Workspace.into()
                ],
                vec![
                    SpaceSidebarToken::Branch.into(),
                    SpaceSidebarToken::GitStatus.into()
                ],
            ]
        );
        assert_eq!(config.spaces.row_gap, 0);
    }

    #[test]
    fn parses_builtin_and_arbitrary_custom_tokens() {
        let config: crate::config::Config = toml::from_str(
            r#"
[ui.sidebar.agents]
rows = [["state_icon", "workspace"], ["state_text", "agent", "$summary"], ["terminal_title", "terminal_title_stripped", "$terminal_title"]]
row_gap = 1

[ui.sidebar.agents.rows_by_agent]
claude = [["terminal_title_stripped"], ["agent", "$model"]]

[ui.sidebar.spaces]
rows = [["workspace"], ["$jj_status"]]
row_gap = 3
"#,
        )
        .expect("sidebar token config");

        assert_eq!(
            config.ui.sidebar.agents.rows[1],
            vec![
                AgentSidebarToken::StateText.into(),
                AgentSidebarToken::Agent.into(),
                AgentSidebarToken::Custom("summary".into()).into(),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[2],
            vec![
                AgentSidebarToken::TerminalTitle.into(),
                AgentSidebarToken::TerminalTitleStripped.into(),
                AgentSidebarToken::Custom("terminal_title".into()).into(),
            ]
        );
        assert_eq!(
            config.ui.sidebar.agents.rows_by_agent["claude"],
            vec![
                vec![AgentSidebarToken::TerminalTitleStripped.into()],
                vec![
                    AgentSidebarToken::Agent.into(),
                    AgentSidebarToken::Custom("model".into()).into(),
                ],
            ]
        );
        assert_eq!(config.ui.sidebar.agents.row_gap, 1);
        assert_eq!(
            config.ui.sidebar.spaces.rows[1],
            vec![SpaceSidebarToken::Custom("jj_status".into()).into()]
        );
        assert_eq!(config.ui.sidebar.spaces.row_gap, 3);
    }

    #[test]
    fn parses_occurrence_styles_without_changing_plain_tokens() {
        let config: crate::config::Config = toml::from_str(
            r##"
[ui.sidebar.agents]
rows = [[{ token = "workspace", fg = "#abc", bold = false }, "workspace"], [{ token = "$summary", dim = false }]]

[ui.sidebar.agents.rows_by_agent]
claude = [[{ token = "agent", fg = "#112233", bold = true, dim = false }]]

[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#ff00aa" }], [{ token = "$jj", bold = true }]]
"##,
        )
        .unwrap();

        let workspace = &config.ui.sidebar.agents.rows[0][0];
        assert_eq!(workspace.token, AgentSidebarToken::Workspace);
        assert_eq!(workspace.style.bold, Some(false));
        assert_eq!(
            workspace.style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xaa, 0xbb, 0xcc)
        );
        assert_eq!(
            config.ui.sidebar.agents.rows[0][1],
            AgentSidebarToken::Workspace.into()
        );

        let agent = &config.ui.sidebar.agents.rows_by_agent["claude"][0][0];
        assert_eq!(agent.token, AgentSidebarToken::Agent);
        assert_eq!(agent.style.bold, Some(true));
        assert_eq!(agent.style.dim, Some(false));

        let git_status = &config.ui.sidebar.spaces.rows[0][0];
        assert_eq!(git_status.token, SpaceSidebarToken::GitStatus);
        assert_eq!(
            git_status.style.fg.unwrap().ratatui(),
            ratatui::style::Color::Rgb(0xff, 0x00, 0xaa)
        );
        let custom = &config.ui.sidebar.spaces.rows[1][0];
        assert_eq!(custom.token, SpaceSidebarToken::Custom("jj".into()));
        assert_eq!(custom.style.bold, Some(true));
    }

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
        assert_eq!(config.agents.rows[0][0].separator_before, None);
        assert_eq!(
            config.agents.rows[0][1].separator_before.as_deref(),
            Some("")
        );
        assert_eq!(
            config.agents.rows_by_agent["pi"][0][0]
                .separator_before
                .as_deref(),
            Some(" / ")
        );
        assert_eq!(
            config.spaces.rows[0][1].separator_before.as_deref(),
            Some(" |")
        );
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<SidebarConfig>(&encoded).unwrap(), config);
        assert!(!toml::to_string(&SidebarConfig::default())
            .unwrap()
            .contains("separator_before"));
    }

    #[test]
    fn conditional_sidebar_rules_round_trip() {
        let input = r##"
[agents]
rows = [[{ token = "machine", fg = "#fff", rules = [{ equals = "Local", fg = "#f00" }, { starts_with = "fed", ignore_case = true, bold = true }] }]]
[agents.rows_by_agent]
pi = [[{ token = "$load", rules = [{ gt = 80, dim = false }, { lt = 20.5, dim = true }] }]]
[spaces]
rows = [[{ token = "$status", rules = [{ contains = "error", bold = true }] }]]
"##;
        let config: SidebarConfig = toml::from_str(input).expect("conditional sidebar config");
        let encoded = toml::to_string(&config).unwrap();
        assert!(encoded.contains("rules"));
        assert_eq!(toml::from_str::<SidebarConfig>(&encoded).unwrap(), config);
    }

    #[test]
    fn conditional_sidebar_rules_reject_invalid_conditions_and_nontext_tokens() {
        for rule in [
            "{ bold = true }",
            "{ equals = 'x', contains = 'x' }",
            "{ regex = 'x' }",
            "{ gt = '80' }",
            "{ equals = 80 }",
            "{ gt = nan }",
            "{ lt = inf }",
            "{ gt = 80, ignore_case = false }",
            "{ equals = 'x', underline = true }",
            "{ equals = 'x', fg = 'red' }",
        ] {
            let input = format!("[agents]\nrows = [[{{ token = 'machine', rules = [{rule}] }}]]");
            assert!(toml::from_str::<SidebarConfig>(&input).is_err(), "{rule}");
        }
        for (section, token) in [
            ("agents", "state_icon"),
            ("spaces", "state_icon"),
            ("spaces", "git_status"),
        ] {
            let input = format!(
                "[{section}]\nrows = [[{{ token = '{token}', rules = [{{ equals = 'x' }}] }}]]"
            );
            assert!(toml::from_str::<SidebarConfig>(&input).is_err());
        }
        for count in [16, 17] {
            let rules = std::iter::repeat_n("{ equals = 'x' }", count)
                .collect::<Vec<_>>()
                .join(",");
            let input = format!("[agents]\nrows = [[{{ token = 'machine', rules = [{rules}] }}]]");
            assert_eq!(toml::from_str::<SidebarConfig>(&input).is_ok(), count == 16);
        }
    }

    #[test]
    fn rejects_invalid_occurrence_styles() {
        for entry in [
            r##"{ token = "workspace", fg = "red" }"##,
            r##"{ token = "workspace", fg = "#abcd" }"##,
            r##"{ token = "workspace", underline = true }"##,
        ] {
            let input = format!("[ui.sidebar.agents]\nrows = [[{entry}]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted {entry}"
            );
        }
    }

    #[test]
    fn rejects_unknown_bare_and_malformed_custom_tokens() {
        for token in ["summary", "$", "$bad.name"] {
            let input = format!("[ui.sidebar.agents]\\nrows = [[\"{token}\"]]\\n");
            assert!(toml::from_str::<crate::config::Config>(&input).is_err());
        }
    }

    #[test]
    fn rejects_oversized_sidebar_layouts() {
        let too_many_rows = std::iter::repeat_n("[\"agent\"]", MAX_SIDEBAR_ROWS + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.agents]\nrows = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let too_many_tokens = std::iter::repeat_n("\"workspace\"", MAX_SIDEBAR_TOKENS_PER_ROW + 1)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!("[ui.sidebar.spaces]\nrows = [[{too_many_tokens}]]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());

        let input = format!("[ui.sidebar.agents.rows_by_agent]\nclaude = [{too_many_rows}]\n");
        assert!(toml::from_str::<crate::config::Config>(&input).is_err());
    }

    #[test]
    fn accepts_every_canonical_agent_override_key() {
        let agents = Agent::ALL;
        let entries = agents
            .iter()
            .map(|agent| format!("{} = [[\"agent\"]]", crate::detect::agent_label(*agent)))
            .collect::<Vec<_>>()
            .join("\n");
        let input = format!("[ui.sidebar.agents.rows_by_agent]\n{entries}\n");
        let config: crate::config::Config = toml::from_str(&input).expect("canonical keys");

        assert_eq!(config.ui.sidebar.agents.rows_by_agent.len(), agents.len());
    }

    #[test]
    fn rejects_alias_case_whitespace_and_unknown_override_keys() {
        for key in ["claude-code", "Claude", "' claude '", "unknown"] {
            let input = format!("[ui.sidebar.agents.rows_by_agent]\n{key} = [[\"agent\"]]\n");
            assert!(
                toml::from_str::<crate::config::Config>(&input).is_err(),
                "accepted key {key:?}"
            );
        }
    }
}
