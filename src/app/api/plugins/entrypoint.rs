//! Resolve a plugin entrypoint to the action or pane of an enabled plugin on
//! this platform. Actions and panes share one id namespace per plugin, so
//! `plugin_id.entrypoint` names exactly one of them. Callers reload the plugin
//! registry before resolving.

use super::{
    effective_platforms, ensure_platform_supported, manifest_action_info, normalize_action_id,
    normalize_plugin_id, plugin_manifest_available,
};
use crate::api::schema::{
    ErrorBody, InstalledPluginInfo, PluginActionInfo, PluginManifestPane, PluginPanePlacement,
};
use crate::app::App;

pub(super) enum PluginEntrypoint {
    Action(PluginActionInfo),
    Pane(PluginManifestPane),
}

pub(super) fn plugin_error(code: &str, message: impl Into<String>) -> ErrorBody {
    ErrorBody {
        code: code.to_owned(),
        message: message.into(),
    }
}

impl App {
    pub(super) fn resolve_plugin_entrypoint(
        &self,
        plugin_id: &str,
        entrypoint: &str,
    ) -> Result<(InstalledPluginInfo, PluginEntrypoint), ErrorBody> {
        let plugin_id = normalize_plugin_id(plugin_id)
            .ok_or_else(|| plugin_error("invalid_plugin_id", "invalid plugin id"))?;
        let entrypoint = normalize_action_id(entrypoint)
            .ok_or_else(|| plugin_error("invalid_plugin_entrypoint", "invalid entrypoint id"))?;
        let plugin = self
            .state
            .installed_plugins
            .get(&plugin_id)
            .cloned()
            .ok_or_else(|| plugin_error("plugin_not_found", "plugin not found"))?;
        if !plugin_manifest_available(&plugin) {
            return Err(plugin_error(
                "plugin_manifest_unavailable",
                format!("plugin {plugin_id} manifest is unavailable"),
            ));
        }
        if !plugin.enabled {
            return Err(plugin_error(
                "plugin_disabled",
                format!("plugin {plugin_id} is disabled"),
            ));
        }
        let (resolved, platforms) =
            if let Some(action) = plugin.actions.iter().find(|a| a.id == entrypoint) {
                (
                    PluginEntrypoint::Action(manifest_action_info(
                        &plugin_id,
                        &plugin.platforms,
                        action,
                    )),
                    &action.platforms,
                )
            } else if let Some(pane) = plugin.panes.iter().find(|p| p.id == entrypoint) {
                (PluginEntrypoint::Pane(pane.clone()), &pane.platforms)
            } else {
                return Err(plugin_error(
                    "plugin_entrypoint_not_found",
                    format!("plugin entrypoint {plugin_id}.{entrypoint} not found"),
                ));
            };
        ensure_platform_supported(
            effective_platforms(platforms, &plugin.platforms),
            &format!("{plugin_id}.{entrypoint}"),
        )
        .map_err(|(code, message)| plugin_error(code, message))?;
        Ok((plugin, resolved))
    }

    /// Resolve a qualified `plugin_id.entrypoint`, splitting at the last dot
    /// because plugin ids may contain dots and entrypoint ids cannot.
    pub(super) fn resolve_qualified_plugin_entrypoint(
        &self,
        target: &str,
    ) -> Result<(InstalledPluginInfo, PluginEntrypoint), ErrorBody> {
        let (plugin_id, entrypoint) = target.rsplit_once('.').ok_or_else(|| {
            plugin_error(
                "invalid_plugin_entrypoint",
                "expected a qualified plugin_id.entrypoint",
            )
        })?;
        self.resolve_plugin_entrypoint(plugin_id, entrypoint)
    }

    /// Whether a qualified target names a popup pane in the loaded registry.
    /// The client uses this to hold input until the popup surface arrives.
    pub(crate) fn plugin_target_opens_popup(&self, target: &str) -> bool {
        target
            .rsplit_once('.')
            .and_then(|(plugin_id, entrypoint)| {
                self.state
                    .installed_plugins
                    .get(plugin_id.trim())?
                    .panes
                    .iter()
                    .find(|pane| pane.id == entrypoint.trim())
            })
            .is_some_and(|pane| pane.placement == PluginPanePlacement::Popup)
    }
}
