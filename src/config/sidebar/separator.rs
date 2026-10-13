//! `separator_before` on a sidebar token table replaces the row's separator
//! before that token. Herdr GPUI reads the same option in
//! `crates/herdr-gpui/src/config/sidebar.rs`.

use std::sync::Arc;

use super::{AgentSidebarToken, SpaceSidebarToken};

pub(super) type Separator = Option<Arc<str>>;

/// Control characters would break the row's layout, so they are dropped.
pub(super) fn sanitize(separator: Option<String>) -> Separator {
    separator.map(|separator| {
        separator
            .chars()
            .filter(|ch| !ch.is_control())
            .collect::<String>()
            .into()
    })
}

impl AgentSidebarToken {
    pub(crate) fn separator_before(&self) -> Separator {
        match self {
            Self::Styled {
                separator_before, ..
            } => separator_before.clone(),
            _ => None,
        }
    }
}

impl SpaceSidebarToken {
    pub(crate) fn separator_before(&self) -> Separator {
        match self {
            Self::Styled {
                separator_before, ..
            } => separator_before.clone(),
            _ => None,
        }
    }
}
