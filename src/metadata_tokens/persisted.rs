//! Tokens saved with the session, so they survive restarts and handoffs.

use std::collections::HashMap;

use super::MetadataTokens;

impl MetadataTokens {
    /// Patches tokens that never expire and are saved with the session.
    pub(crate) fn patch_persisted(&mut self, patch: HashMap<String, Option<String>>) -> bool {
        self.apply(patch, None, true)
    }

    /// Tokens restored from a saved session.
    pub(crate) fn restored(values: HashMap<String, String>) -> Self {
        let mut tokens = Self::default();
        tokens.patch_persisted(
            values
                .into_iter()
                .map(|(key, value)| (key, Some(value)))
                .collect(),
        );
        tokens
    }

    /// The tokens to save with the session.
    pub(crate) fn persisted(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .filter(|(_, token)| token.persist)
            .map(|(key, token)| (key.clone(), token.value.clone()))
            .collect()
    }
}
