//! What the fork keeps per app, and per app state, beside upstream's fields.

/// The fork's fields of [`super::App`].
#[derive(Default)]
pub(crate) struct AppFork {
    /// The shell client whose request is being handled, set only for the
    /// duration of that request. Plugins started by it record their origin.
    pub(crate) invoking_client_id: Option<u64>,
    /// The target context of the `plugin.invoke` call whose pane is opening,
    /// set only while that call runs the `plugin.pane.open` handler.
    pub(crate) invoked_plugin_context: Option<crate::api::schema::PluginInvocationContext>,
}

/// The fork's fields of [`super::state::AppState`].
#[derive(Default)]
pub(crate) struct StateFork {
    /// Whether some pane may carry an unread hold. Lets the server skip its
    /// per-event settle pass; it clears the flag once it finds none.
    pub(crate) unread_holds_pending: bool,
}
