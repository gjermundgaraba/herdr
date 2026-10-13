//! Resolve and invoke plugin entrypoints. Actions and panes share one id
//! namespace per plugin, so `plugin_id.entrypoint` names exactly one of them.
//! Keys and `plugin.invoke` both run entrypoints through
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
    ) -> Result<(InstalledPluginInfo, PluginEntrypoint), (&'static str, String)> {
        let plugin_id = normalize_plugin_id(plugin_id)
            .ok_or_else(|| ("invalid_plugin_id", "invalid plugin id".to_owned()))?;
        let entrypoint = normalize_action_id(entrypoint).ok_or_else(|| {
            (
                "invalid_plugin_entrypoint",
                "invalid entrypoint id".to_owned(),
            )
        })?;
        let plugin = self
            .state
            .installed_plugins
            .get(&plugin_id)
            .cloned()
            .ok_or_else(|| ("plugin_not_found", "plugin not found".to_owned()))?;
        if !plugin_manifest_available(&plugin) {
            return Err((
                "plugin_manifest_unavailable",
                format!("plugin {plugin_id} manifest is unavailable"),
            ));
        }
        if !plugin.enabled {
            return Err(("plugin_disabled", format!("plugin {plugin_id} is disabled")));
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
                return Err((
                    "plugin_entrypoint_not_found",
                    format!("plugin entrypoint {plugin_id}.{entrypoint} not found"),
                ));
            };
        ensure_platform_supported(
            effective_platforms(platforms, &plugin.platforms),
            &format!("{plugin_id}.{entrypoint}"),
        )?;
        Ok((plugin, resolved))
    }

    /// Resolve a qualified `plugin_id.entrypoint`, splitting at the last dot
    /// because plugin ids may contain dots and entrypoint ids cannot.
    pub(super) fn resolve_qualified_plugin_entrypoint(
        &self,
        target: &str,
    ) -> Result<(InstalledPluginInfo, PluginEntrypoint), (&'static str, String)> {
        let (plugin_id, entrypoint) = target.rsplit_once('.').ok_or_else(|| {
            (
                "invalid_plugin_entrypoint",
                "expected a qualified plugin_id.entrypoint".to_owned(),
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
        let resolved = self
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
                Ok((plugin, entrypoint, context))
            });
        match resolved {
            Ok((plugin, entrypoint, context)) => {
                self.invoke_plugin_entrypoint(id, plugin, entrypoint, context, None, None)
            }
            Err((code, message)) => encode_error(id, code, message),
        }
    }

    /// Context for the target of a `plugin.invoke` request. The most
    /// specific id supplies it, so the invocation describes what it targets,
    /// not what has focus; less specific ids must agree. Without ids, focus
    /// supplies it, as it does for a keybinding, which names no target.
    pub(super) fn plugin_context_for_target(
        &self,
        workspace_id: Option<&str>,
        tab_id: Option<&str>,
        pane_id: Option<&str>,
        correlation_id: &str,
    ) -> Result<PluginInvocationContext, (&'static str, String)> {
        let context = if let Some(pane_id) = pane_id {
            self.plugin_context_for_public_pane_id(pane_id, correlation_id)
                .ok_or_else(|| ("pane_not_found", format!("pane not found: {pane_id}")))?
        } else if let Some(tab_id) = tab_id {
            self.plugin_context_for_tab_id(tab_id, correlation_id)
                .ok_or_else(|| ("tab_not_found", format!("tab not found: {tab_id}")))?
        } else if let Some(workspace_id) = workspace_id {
            self.plugin_context_for_workspace_id(workspace_id, correlation_id)
                .ok_or_else(|| {
                    (
                        "workspace_not_found",
                        format!("workspace not found: {workspace_id}"),
                    )
                })?
        } else {
            self.current_plugin_context(correlation_id)
        };
        let disagrees = |provided: Option<&str>, actual: &Option<String>| {
            provided.is_some_and(|provided| actual.as_deref() != Some(provided))
        };
        if disagrees(workspace_id, &context.workspace_id) || disagrees(tab_id, &context.tab_id) {
            return Err((
                "target_mismatch",
                "target ids do not name the same space, tab, and pane".to_owned(),
            ));
        }
        Ok(context)
    }

    /// Reads a client selection, which must lie in the target pane.
    fn plugin_selection_text(
        &self,
        pane_id: Option<&str>,
        selection: Option<&PaneSelectionReadParams>,
    ) -> Result<Option<String>, (&'static str, String)> {
        let Some(selection) = selection else {
            return Ok(None);
        };
        if pane_id != Some(selection.pane_id.as_str()) {
            return Err((
                "target_mismatch",
                "selection does not belong to the target pane".to_owned(),
            ));
        }
        self.pane_selection_text(selection).map(Some)
    }

    /// Runs an action, or opens a pane at its manifest placement through
    /// `plugin.pane.open`. The context names the target: split and zoomed
    /// panes split its pane, tab panes open in its space, and the pane
    /// launches with it. Sizes override a popup's manifest size; other
    /// placements take none.
    pub(super) fn invoke_plugin_entrypoint(
        &mut self,
        id: String,
        plugin: InstalledPluginInfo,
        entrypoint: PluginEntrypoint,
        context: PluginInvocationContext,
        width: Option<PopupSize>,
        height: Option<PopupSize>,
    ) -> String {
        let pane = match entrypoint {
            PluginEntrypoint::Action(action) => {
                return match self.start_plugin_command(
                    &plugin,
                    Some(action.action_id.clone()),
                    None,
                    action.command.clone(),
                    &context,
                    None,
                ) {
                    Ok(log) => encode_success(
                        id,
                        ResponseResult::PluginActionInvoked {
                            action,
                            context,
                            log,
                        },
                    ),
                    Err((code, message)) => encode_error(id, code, message),
                };
            }
            PluginEntrypoint::Pane(pane) => pane,
        };
        let (workspace_id, target_pane_id) = match pane.placement {
            PluginPanePlacement::Split | PluginPanePlacement::Zoomed => {
                (None, context.focused_pane_id.clone())
            }
            PluginPanePlacement::Tab => (context.workspace_id.clone(), None),
            PluginPanePlacement::Popup | PluginPanePlacement::Overlay => (None, None),
        };
        let popup = pane.placement == PluginPanePlacement::Popup;
        let params = PluginPaneOpenParams {
            plugin_id: plugin.plugin_id,
            entrypoint: pane.id,
            placement: None,
            width: width.filter(|_| popup),
            height: height.filter(|_| popup),
            workspace_id,
            target_pane_id,
            direction: None,
            cwd: None,
            focus: true,
            env: Default::default(),
        };
        self.invoked_plugin_context = Some(context);
        let response = self.handle_plugin_pane_open(id, params);
        self.invoked_plugin_context = None;
        response
    }

    /// The context a plugin pane launches with: that of the invoked target
    /// while `plugin.invoke` opens it, otherwise the one `plugin.pane.open`
    /// resolved.
    pub(super) fn invoked_pane_context(
        &self,
        context: &PluginInvocationContext,
    ) -> PluginInvocationContext {
        self.with_invocation_details(
            self.invoked_plugin_context
                .clone()
                .unwrap_or_else(|| context.clone()),
        )
    }

    /// Adds what an invoked plugin would otherwise query next: the invoking
    /// client, the context workspace's tabs, and every workspace. Event hooks
    /// skip this; they fire per event and their event JSON names the entities.
    pub(super) fn with_invocation_details(
        &self,
        mut context: PluginInvocationContext,
    ) -> PluginInvocationContext {
        context.client_id = self.invoking_client_id.or(context.client_id);
        if let Some((ws_idx, ws)) = context
            .workspace_id
            .as_deref()
            .and_then(|workspace_id| self.parse_workspace_id(workspace_id))
            .and_then(|ws_idx| Some((ws_idx, self.state.workspaces.get(ws_idx)?)))
        {
            context.tabs = (0..ws.tabs.len())
                .filter_map(|tab_idx| self.tab_info(ws_idx, tab_idx))
                .collect();
        }
        context.workspaces = (0..self.state.workspaces.len())
            .map(|ws_idx| self.workspace_info(ws_idx))
            .collect();
        context
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
