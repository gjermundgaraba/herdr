//! Pinned sections of the spaces sidebar: `[[ui.sidebar.spaces.pinned]]`.
//!
//! Each section mirrors every space whose workspace metadata carries the
//! section's token with a non-empty value, optionally narrowed by one text
//! condition. The client shell draws them above the regular space list.

use serde::{Deserialize, Serialize};

use super::SidebarTokenColor;

pub(super) const MAX_PINNED_SECTIONS: usize = 16;

pub(super) type PinnedSpaceSections = Vec<PinnedSpaceSection>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawPinnedSection", into = "RawPinnedSection")]
pub struct PinnedSpaceSection {
    pub title: String,
    pub token: String,
    condition: Option<PinnedCondition>,
    pub fg: Option<SidebarTokenColor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PinnedCondition {
    Equals(String),
    Contains(String),
    StartsWith(String),
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawPinnedSection {
    title: String,
    token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    equals: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contains: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    starts_with: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fg: Option<SidebarTokenColor>,
}

impl TryFrom<RawPinnedSection> for PinnedSpaceSection {
    type Error = String;

    fn try_from(raw: RawPinnedSection) -> Result<Self, Self::Error> {
        if raw.title.trim().is_empty() {
            return Err("pinned space section requires a non-empty title".into());
        }
        if raw.token.trim().is_empty() {
            return Err(format!(
                "pinned space section {:?} requires a non-empty token",
                raw.title
            ));
        }
        let condition = match (raw.equals, raw.contains, raw.starts_with) {
            (None, None, None) => None,
            (Some(value), None, None) => Some(PinnedCondition::Equals(value)),
            (None, Some(value), None) => Some(PinnedCondition::Contains(value)),
            (None, None, Some(value)) => Some(PinnedCondition::StartsWith(value)),
            _ => {
                return Err(format!(
                    "pinned space section {:?} accepts at most one of equals, contains, starts_with",
                    raw.title
                ));
            }
        };
        Ok(Self {
            title: raw.title,
            token: raw.token,
            condition,
            fg: raw.fg,
        })
    }
}

impl From<PinnedSpaceSection> for RawPinnedSection {
    fn from(section: PinnedSpaceSection) -> Self {
        let mut raw = Self {
            title: section.title,
            token: section.token,
            fg: section.fg,
            ..Self::default()
        };
        match section.condition {
            Some(PinnedCondition::Equals(value)) => raw.equals = Some(value),
            Some(PinnedCondition::Contains(value)) => raw.contains = Some(value),
            Some(PinnedCondition::StartsWith(value)) => raw.starts_with = Some(value),
            None => {}
        }
        raw
    }
}

impl PinnedSpaceSection {
    /// Whether a space with these metadata tokens belongs in the section: it
    /// carries the token with a non-empty value that meets the condition.
    pub(crate) fn matches<'a>(&self, mut tokens: impl Iterator<Item = (&'a str, &'a str)>) -> bool {
        let Some((_, value)) = tokens.find(|(key, _)| *key == self.token) else {
            return false;
        };
        if value.is_empty() {
            return false;
        }
        match &self.condition {
            None => true,
            Some(PinnedCondition::Equals(expected)) => value == expected,
            Some(PinnedCondition::Contains(expected)) => value.contains(expected.as_str()),
            Some(PinnedCondition::StartsWith(expected)) => value.starts_with(expected.as_str()),
        }
    }
}

pub(super) fn deserialize_pinned_sections<'de, D>(
    deserializer: D,
) -> Result<PinnedSpaceSections, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let sections = PinnedSpaceSections::deserialize(deserializer)?;
    validate_pinned_sections(&sections).map_err(serde::de::Error::custom)?;
    Ok(sections)
}

fn validate_pinned_sections(sections: &[PinnedSpaceSection]) -> Result<(), String> {
    if sections.len() > MAX_PINNED_SECTIONS {
        return Err(format!(
            "ui.sidebar.spaces.pinned may contain at most {MAX_PINNED_SECTIONS} sections"
        ));
    }
    for (index, section) in sections.iter().enumerate() {
        if sections[..index]
            .iter()
            .any(|earlier| earlier.title == section.title)
        {
            return Err(format!(
                "pinned space section title {:?} is used more than once",
                section.title
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::SidebarConfig;
    use super::*;

    fn sections(input: &str) -> Result<PinnedSpaceSections, String> {
        toml::from_str::<SidebarConfig>(input)
            .map(|config| config.spaces.pinned)
            .map_err(|error| error.to_string())
    }

    fn matches(section: &PinnedSpaceSection, tokens: &[(&str, &str)]) -> bool {
        section.matches(tokens.iter().copied())
    }

    #[test]
    fn parses_sections_in_order_and_round_trips() {
        let input = r##"
[[spaces.pinned]]
title = "Priority"
token = "priority"
fg = "#e5c07b"

[[spaces.pinned]]
title = "Review"
token = "stage"
equals = "review"
"##;
        let config: SidebarConfig = toml::from_str(input).expect("pinned sections");
        let pinned = &config.spaces.pinned;
        assert_eq!(pinned.len(), 2);
        assert_eq!(pinned[0].title, "Priority");
        assert_eq!(pinned[0].token, "priority");
        assert_eq!(
            pinned[0].fg.map(SidebarTokenColor::ratatui),
            Some(ratatui::style::Color::Rgb(0xe5, 0xc0, 0x7b))
        );
        assert_eq!(pinned[1].title, "Review");
        assert_eq!(pinned[1].fg, None);
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<SidebarConfig>(&encoded).unwrap(), config);
        assert!(!toml::to_string(&SidebarConfig::default())
            .unwrap()
            .contains("pinned"));
    }

    #[test]
    fn matching_requires_a_non_empty_value_that_meets_the_condition() {
        let any = &sections("[[spaces.pinned]]\ntitle = 'P'\ntoken = 'prio'").unwrap()[0];
        assert!(matches(any, &[("other", "x"), ("prio", "1")]));
        assert!(!matches(any, &[("prio", "")]));
        assert!(!matches(any, &[("other", "1")]));
        assert!(!matches(any, &[]));

        for (condition, yes, no) in [
            ("equals = 'high'", "high", "higher"),
            ("contains = 'ig'", "high", "low"),
            ("starts_with = 'hi'", "high", "ship"),
        ] {
            let input = format!("[[spaces.pinned]]\ntitle = 'P'\ntoken = 'prio'\n{condition}");
            let section = &sections(&input).unwrap()[0];
            assert!(matches(section, &[("prio", yes)]), "{condition}");
            assert!(!matches(section, &[("prio", no)]), "{condition}");
            assert!(!matches(section, &[("prio", "")]), "{condition}");
            assert!(!matches(section, &[("prio", "HIGH")]), "{condition}");
        }
    }

    #[test]
    fn rejects_invalid_sections() {
        for (input, message) in [
            ("token = 'prio'", "non-empty title"),
            ("title = ' '\ntoken = 'prio'", "non-empty title"),
            ("title = 'P'", "non-empty token"),
            ("title = 'P'\ntoken = ''", "non-empty token"),
            (
                "title = 'P'\ntoken = 'prio'\nequals = 'a'\ncontains = 'b'",
                "at most one of",
            ),
            ("title = 'P'\ntoken = 'prio'\nfg = 'red'", "#RGB or #RRGGBB"),
            ("title = 'P'\ntoken = 'prio'\ngt = 1", "unknown field"),
            (
                "title = 'P'\ntoken = 'prio'\nignore_case = true",
                "unknown field",
            ),
        ] {
            let error = sections(&format!("[[spaces.pinned]]\n{input}")).unwrap_err();
            assert!(error.contains(message), "{input}: {error}");
        }

        let duplicate = "[[spaces.pinned]]\ntitle = 'P'\ntoken = 'a'\n\
                         [[spaces.pinned]]\ntitle = 'P'\ntoken = 'b'";
        assert!(sections(duplicate)
            .unwrap_err()
            .contains("used more than once"));

        for count in [MAX_PINNED_SECTIONS, MAX_PINNED_SECTIONS + 1] {
            let input = (0..count)
                .map(|index| format!("[[spaces.pinned]]\ntitle = 'P{index}'\ntoken = 'prio'\n"))
                .collect::<String>();
            assert_eq!(
                sections(&input).is_ok(),
                count == MAX_PINNED_SECTIONS,
                "{count}"
            );
        }
    }
}
