use ratatui::layout::Direction;

use super::entrypoint::plugin_error;
use crate::api::schema::{
    ErrorBody, InstalledPluginInfo, PluginInvocationContext, PluginManifestPane, PluginPaneInfo,
    PluginPaneOpenParams, PluginPanePlacement, ResponseResult, SplitDirection,
};
use crate::app::App;
use crate::layout::PaneId;

enum PaneTarget {
    Popup,
    Overlay,
    Split {
        workspace: usize,
        pane: PaneId,
        zoomed: bool,
        direction: Direction,
    },
    Tab {
        workspace: usize,
    },
}

struct PaneLaunch {
    plugin_id: String,
    pane: PluginManifestPane,
    cwd: std::path::PathBuf,
    env: Vec<(String, String)>,
}

impl App {
    /// Open a resolved pane entrypoint. Request sizes apply only to popups.
    /// An invocation `context` describes its target; without one, the
    /// placement target supplies it.
    pub(super) fn open_plugin_pane(
        &mut self,
        plugin: InstalledPluginInfo,
        mut pane: PluginManifestPane,
        params: PluginPaneOpenParams,
        context: Option<PluginInvocationContext>,
    ) -> Result<ResponseResult, ErrorBody> {
        let PluginPaneOpenParams {
            placement,
            width,
            height,
            workspace_id,
            target_pane_id,
            direction,
            cwd,
            focus,
            env,
            ..
        } = params;
        let target = self.resolve_plugin_pane_target(
            placement.unwrap_or(pane.placement),
            workspace_id.as_deref(),
            target_pane_id.as_deref(),
            direction,
        )?;
        let context = context.unwrap_or_else(|| match target {
            PaneTarget::Popup | PaneTarget::Overlay => self.current_plugin_context("plugin-pane"),
            PaneTarget::Split {
                workspace, pane, ..
            } => self.plugin_context_for_pane(workspace, pane, "plugin-pane"),
            PaneTarget::Tab { workspace } => {
                self.plugin_context_for_workspace(workspace, "plugin-pane")
            }
        });
        pane.command[0] = crate::plugin_command::program_for_cwd(
            &pane.command[0],
            std::path::Path::new(&plugin.plugin_root),
        )
        .display()
        .to_string();
        let cwd = cwd
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(&plugin.plugin_root));
        let env = self
            .plugin_pane_launch_env(&plugin, &pane.id, &cwd, env, &context)
            .map_err(|(code, message)| plugin_error(&code, message))?;
        let geometry = crate::app::popup::PopupGeometry {
            width: width.or(pane.width),
            height: height.or(pane.height),
        };
        let launch = PaneLaunch {
            plugin_id: plugin.plugin_id,
            pane,
            cwd,
            env,
        };
        match target {
            PaneTarget::Popup => self.open_plugin_popup_pane(geometry, launch),
            PaneTarget::Overlay => self.open_plugin_overlay_pane(launch),
            PaneTarget::Split {
                workspace,
                pane,
                zoomed,
                direction,
            } => self.open_plugin_split_pane(workspace, pane, zoomed, direction, focus, launch),
            PaneTarget::Tab { workspace } => self.open_plugin_tab(workspace, focus, launch),
        }
    }

    fn resolve_plugin_pane_target(
        &self,
        placement: PluginPanePlacement,
        workspace_id: Option<&str>,
        target_pane_id: Option<&str>,
        direction: Option<SplitDirection>,
    ) -> Result<PaneTarget, ErrorBody> {
        match placement {
            PluginPanePlacement::Popup | PluginPanePlacement::Overlay => {
                if placement == PluginPanePlacement::Popup && self.state.popup_pane.is_some() {
                    return Err(plugin_error("ui_busy", "a popup pane is already open"));
                }
                if workspace_id.is_some() || target_pane_id.is_some() || direction.is_some() {
                    return Err(plugin_error(
                        "invalid_params",
                        "overlay and popup plugin panes target the active pane",
                    ));
                }
                Ok(if placement == PluginPanePlacement::Popup {
                    PaneTarget::Popup
                } else {
                    PaneTarget::Overlay
                })
            }
            PluginPanePlacement::Split | PluginPanePlacement::Zoomed => {
                if workspace_id.is_some() {
                    return Err(plugin_error(
                        "invalid_params",
                        "split and zoomed plugin panes target an existing pane; use target_pane_id",
                    ));
                }
                let (workspace, pane) = if let Some(id) = target_pane_id {
                    self.parse_pane_id(id).ok_or_else(|| {
                        plugin_error("pane_not_found", format!("pane {id} not found"))
                    })?
                } else {
                    self.state
                        .active
                        .and_then(|workspace| {
                            self.state
                                .workspaces
                                .get(workspace)
                                .and_then(|ws| ws.focused_pane_id())
                                .map(|pane| (workspace, pane))
                        })
                        .ok_or_else(|| plugin_error("no_active_pane", "no active pane"))?
                };
                Ok(PaneTarget::Split {
                    workspace,
                    pane,
                    zoomed: placement == PluginPanePlacement::Zoomed,
                    direction: match direction.unwrap_or(SplitDirection::Right) {
                        SplitDirection::Right => Direction::Horizontal,
                        SplitDirection::Down => Direction::Vertical,
                    },
                })
            }
            PluginPanePlacement::Tab => {
                if target_pane_id.is_some() || direction.is_some() {
                    return Err(plugin_error(
                        "invalid_params",
                        "tab plugin panes support workspace_id but not target_pane_id or direction",
                    ));
                }
                let workspace = match workspace_id {
                    Some(id) => self.parse_workspace_id(id).ok_or_else(|| {
                        plugin_error("workspace_not_found", "workspace not found")
                    })?,
                    None => self.state.active.ok_or_else(|| {
                        plugin_error("no_active_workspace", "no active workspace")
                    })?,
                };
                Ok(PaneTarget::Tab { workspace })
            }
        }
    }

    fn open_plugin_popup_pane(
        &mut self,
        geometry: crate::app::popup::PopupGeometry,
        launch: PaneLaunch,
    ) -> Result<ResponseResult, ErrorBody> {
        let PaneLaunch { pane, cwd, env, .. } = launch;
        self.spawn_popup_argv_command(&pane.command, Some(cwd), env, geometry)
            .map_err(|err| plugin_error("plugin_pane_open_failed", err.to_string()))?;
        let popup =
            self.state.popup_pane.as_ref().ok_or_else(|| {
                plugin_error("plugin_pane_open_failed", "plugin popup disappeared")
            })?;
        if let Some(terminal) = self.state.terminals.get_mut(&popup.terminal_id) {
            terminal.set_manual_label(pane.title);
        }
        Ok(ResponseResult::Ok {})
    }

    fn open_plugin_overlay_pane(
        &mut self,
        launch: PaneLaunch,
    ) -> Result<ResponseResult, ErrorBody> {
        let PaneLaunch {
            plugin_id,
            pane,
            cwd,
            env,
        } = launch;
        let (ws_idx, new_pane) = self
            .spawn_overlay_argv_command(&pane.command, Some(cwd), env, Vec::new())
            .map_err(|err| plugin_error("plugin_pane_open_failed", err.to_string()))?;
        let layout_tab_idx = self
            .overlay_panes
            .get(&new_pane.pane_id)
            .map(|overlay| overlay.tab_idx);
        self.finish_plugin_pane_open(ws_idx, None, layout_tab_idx, new_pane, plugin_id, pane)
    }

    fn open_plugin_split_pane(
        &mut self,
        ws_idx: usize,
        target_pane: PaneId,
        zoomed: bool,
        direction: Direction,
        focus: bool,
        launch: PaneLaunch,
    ) -> Result<ResponseResult, ErrorBody> {
        let PaneLaunch {
            plugin_id,
            pane,
            cwd,
            env,
        } = launch;
        let (rows, cols) = self.state.estimate_pane_size();
        let previous_focus = self.state.current_pane_focus_target();
        let ws = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| plugin_error("workspace_not_found", "workspace not found"))?;
        let (tab_idx, new_pane) = ws
            .split_pane_argv_command(
                target_pane,
                direction,
                rows.max(4),
                cols.max(10),
                Some(cwd),
                &pane.command,
                env,
                self.state.pane_scrollback_limit_bytes,
                self.state.host_terminal_theme,
                self.state.host_terminal_appearance,
                focus || zoomed,
            )
            .ok_or_else(|| plugin_error("pane_not_found", "target pane not found"))?
            .map_err(|err| plugin_error("plugin_pane_open_failed", err.to_string()))?;
        if focus || zoomed {
            self.state.switch_workspace_tab(ws_idx, tab_idx);
            self.state
                .record_pane_focus_change(previous_focus, ws_idx, new_pane.pane_id);
            self.state.mode = crate::app::Mode::Terminal;
        }
        if zoomed {
            if let Some(tab) = self
                .state
                .workspaces
                .get_mut(ws_idx)
                .and_then(|ws| ws.tabs.get_mut(tab_idx))
            {
                tab.zoomed = true;
            }
        }
        self.finish_plugin_pane_open(ws_idx, None, Some(tab_idx), new_pane, plugin_id, pane)
    }

    fn open_plugin_tab(
        &mut self,
        ws_idx: usize,
        focus: bool,
        launch: PaneLaunch,
    ) -> Result<ResponseResult, ErrorBody> {
        let PaneLaunch {
            plugin_id,
            pane,
            cwd,
            env,
        } = launch;
        let (rows, cols) = self.state.estimate_pane_size();
        let ws = self
            .state
            .workspaces
            .get_mut(ws_idx)
            .ok_or_else(|| plugin_error("workspace_not_found", "workspace not found"))?;
        let (tab_idx, terminal, runtime) = ws
            .create_tab_argv_command(
                rows.max(4),
                cols.max(10),
                cwd,
                &pane.command,
                env,
                self.state.pane_scrollback_limit_bytes,
                self.state.host_terminal_theme,
                self.state.host_terminal_appearance,
            )
            .map_err(|err| plugin_error("plugin_pane_open_failed", err.to_string()))?;
        let pane_id = ws.tabs[tab_idx].root_pane;
        if focus {
            self.state.switch_workspace_tab(ws_idx, tab_idx);
            self.state.mode = crate::app::Mode::Terminal;
        }
        let new_pane = crate::workspace::NewPane {
            pane_id,
            terminal,
            runtime,
        };
        self.finish_plugin_pane_open(
            ws_idx,
            Some(tab_idx),
            Some(tab_idx),
            new_pane,
            plugin_id,
            pane,
        )
    }

    fn plugin_pane_launch_env(
        &self,
        plugin: &InstalledPluginInfo,
        entrypoint: &str,
        cwd: &std::path::Path,
        env: std::collections::HashMap<String, String>,
        context: &PluginInvocationContext,
    ) -> Result<Vec<(String, String)>, (String, String)> {
        let mut env = super::super::env::normalize_launch_env(env)?;
        crate::platform::set_default_plugin_pane_pwd(&mut env, cwd);
        let context = self.with_invocation_details(context.clone());
        let context_json = serde_json::to_string(&context)
            .map_err(|err| ("invalid_plugin_context".to_string(), err.to_string()))?;
        super::env::ensure_plugin_user_dirs(plugin)
            .map_err(|err| ("plugin_user_dir_create_failed".to_string(), err.to_string()))?;
        env.retain(|(key, _)| !plugin_pane_protected_env_key(key));
        env.extend(super::env::plugin_path_env(plugin));
        env.push((
            crate::api::SOCKET_PATH_ENV_VAR.to_string(),
            crate::api::socket_path().display().to_string(),
        ));
        env.push(("HERDR_ENV".to_string(), "1".to_string()));
        env.push(("HERDR_PLUGIN_ID".to_string(), plugin.plugin_id.clone()));
        env.push((
            "HERDR_PLUGIN_ENTRYPOINT_ID".to_string(),
            entrypoint.to_string(),
        ));
        env.push(("HERDR_PLUGIN_CONTEXT_JSON".to_string(), context_json));
        if let Some(client_id) = context.client_id {
            env.push(("HERDR_CLIENT_ID".to_string(), client_id.to_string()));
        }
        if let Ok(current_exe) = crate::platform::launch_executable() {
            env.push((
                "HERDR_BIN_PATH".to_string(),
                current_exe.display().to_string(),
            ));
        }
        Ok(env)
    }

    fn finish_plugin_pane_open(
        &mut self,
        ws_idx: usize,
        created_tab_idx: Option<usize>,
        layout_tab_idx: Option<usize>,
        new_pane: crate::workspace::NewPane,
        plugin_id: String,
        pane_manifest: PluginManifestPane,
    ) -> Result<ResponseResult, ErrorBody> {
        let entrypoint = pane_manifest.id.clone();
        let mut terminal = new_pane.terminal;
        terminal.set_manual_label(pane_manifest.title.clone());
        let terminal_id = terminal.id.clone();
        self.terminal_runtimes
            .insert(terminal_id.clone(), new_pane.runtime);
        self.state
            .remove_alias_shadowed_by_new_pane(new_pane.pane_id);
        self.state.terminals.insert(terminal_id, terminal);
        self.state.plugin_panes.insert(
            new_pane.pane_id,
            crate::app::state::PluginPaneRecord {
                plugin_id: plugin_id.clone(),
                entrypoint: entrypoint.clone(),
            },
        );
        if let Some(tab_idx) = created_tab_idx {
            if let Some(tab) = self.tab_info(ws_idx, tab_idx) {
                self.emit_event(crate::api::schema::EventEnvelope {
                    event: crate::api::schema::EventKind::TabCreated,
                    data: crate::api::schema::EventData::TabCreated { tab },
                });
            }
        }
        self.schedule_session_save();
        let Some(pane) = self.pane_info(ws_idx, new_pane.pane_id) else {
            return Err(plugin_error(
                "plugin_pane_open_failed",
                "plugin pane disappeared",
            ));
        };
        self.emit_event(crate::api::schema::EventEnvelope {
            event: crate::api::schema::EventKind::PaneCreated,
            data: crate::api::schema::EventData::PaneCreated { pane: pane.clone() },
        });
        if let Some(tab_idx) = layout_tab_idx {
            self.emit_layout_updated_event(ws_idx, tab_idx);
        }
        Ok(ResponseResult::PluginPaneOpened {
            plugin_pane: PluginPaneInfo {
                plugin_id,
                entrypoint,
                pane,
            },
        })
    }
}

fn plugin_pane_protected_env_key(key: &str) -> bool {
    matches!(
        key,
        crate::api::SOCKET_PATH_ENV_VAR
            | "HERDR_ENV"
            | "HERDR_PLUGIN_ID"
            | "HERDR_PLUGIN_ROOT"
            | "HERDR_PLUGIN_CONFIG_DIR"
            | "HERDR_PLUGIN_STATE_DIR"
            | "HERDR_PLUGIN_ENTRYPOINT_ID"
            | "HERDR_PLUGIN_CONTEXT_JSON"
            | "HERDR_BIN_PATH"
            | "HERDR_CLIENT_ID"
    )
}
