//! A token's configured `separator_before`, which replaces the row's spacing
//! before it. Herdr GPUI resolves it in
//! `crates/herdr-gpui/src/sidebar/tokens.rs`.

use std::sync::Arc;

use super::ResolvedToken;

impl ResolvedToken {
    pub(super) fn separated(self, separator_before: Option<Arc<str>>) -> Self {
        Self {
            separator_before,
            ..self
        }
    }
}
