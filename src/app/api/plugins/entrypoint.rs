//! Resolve and invoke plugin entrypoints. Actions and panes share one id
//! namespace per plugin, so `plugin_id.entrypoint` names exactly one of them.
//! Keys, menus, and `plugin.invoke` all run entrypoints through
//! `invoke_plugin_entrypoint`. Callers reload the plugin registry before
//! resolving.

use super::{
    effective_platforms, ensure_platform_supported, manifest_action_info, normalize_action_id,
    normalize_plugin_id, plugin_manifest_available,
};
use crate::api::schema::{
    ErrorBody, InstalledPluginInfo, PaneSelectionReadParams, PluginActionInfo,
    PluginInvocationContext, PluginInvokeParams, PluginManifestPane, PluginPaneOpenParams,
    PluginPanePlacement, ResponseResult,
};
use crate::app::api::responses::{encode_error, encode_success};
use crate::app::App;
use crate::popup_size::PopupSize;

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

    pub(crate) fn handle_plugin_invoke(
        &mut self,
        id: String,
        params: PluginInvokeParams,
    ) -> String {
        if let Err(err) = self.refresh_installed_plugins() {
            return encode_error(id, "plugin_registry_load_failed", err.to_string());
        }
        let result = self
            .resolve_plugin_entrypoint(&params.plugin_id, &params.entry_id)
            .and_then(|(plugin, entrypoint)| {
                let mut context = self.plugin_context_for_target(
                    params.workspace_id.as_deref(),
                    params.tab_id.as_deref(),
                    params.pane_id.as_deref(),
                    &id,
                )?;
                context.selected_text = self
                    .plugin_selection_text(params.pane_id.as_deref(), params.selection.as_ref())?;
                self.invoke_plugin_entrypoint(plugin, entrypoint, context, None, None)
            });
        match result {
            Ok(result) => encode_success(id, result),
            Err(error) => encode_error(id, &error.code, error.message),
        }
    }

    /// Reads a client selection, which must lie in the target pane.
    pub(crate) fn plugin_selection_text(
        &self,
        pane_id: Option<&str>,
        selection: Option<&PaneSelectionReadParams>,
    ) -> Result<Option<String>, ErrorBody> {
        let Some(selection) = selection else {
            return Ok(None);
        };
        if pane_id != Some(selection.pane_id.as_str()) {
            return Err(plugin_error(
                "target_mismatch",
                "selection does not belong to the target pane",
            ));
        }
        self.pane_selection_text(selection)
            .map(Some)
            .map_err(|(code, message)| plugin_error(code, message))
    }

    /// Runs an action, or opens a pane at its manifest placement. The context
    /// names the target: split and zoomed panes split its pane, and tab panes
    /// open in its space. Sizes override a popup's manifest size.
    pub(super) fn invoke_plugin_entrypoint(
        &mut self,
        plugin: InstalledPluginInfo,
        entrypoint: PluginEntrypoint,
        context: PluginInvocationContext,
        width: Option<PopupSize>,
        height: Option<PopupSize>,
    ) -> Result<ResponseResult, ErrorBody> {
        match entrypoint {
            PluginEntrypoint::Action(action) => {
                let log = self
                    .start_plugin_command(
                        &plugin,
                        Some(action.action_id.clone()),
                        None,
                        action.command.clone(),
                        &context,
                        None,
                    )
                    .map_err(|(code, message)| plugin_error(code, message))?;
                Ok(ResponseResult::PluginActionInvoked {
                    action,
                    context,
                    log,
                })
            }
            PluginEntrypoint::Pane(pane) => {
                let (workspace_id, target_pane_id) = match pane.placement {
                    PluginPanePlacement::Split | PluginPanePlacement::Zoomed => {
                        (None, context.focused_pane_id.clone())
                    }
                    PluginPanePlacement::Tab => (context.workspace_id.clone(), None),
                    PluginPanePlacement::Popup | PluginPanePlacement::Overlay => (None, None),
                };
                let params = PluginPaneOpenParams {
                    plugin_id: String::new(),
                    entrypoint: String::new(),
                    placement: None,
                    width,
                    height,
                    workspace_id,
                    target_pane_id,
                    direction: None,
                    cwd: None,
                    focus: true,
                    env: Default::default(),
                };
                self.open_plugin_pane(plugin, pane, params, Some(context))
            }
        }
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
