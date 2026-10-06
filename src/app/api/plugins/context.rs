use crate::api::schema::{EventData, PluginInvocationContext};
use crate::app::App;

impl App {
    pub(super) fn merge_plugin_context(
        &self,
        provided: Option<PluginInvocationContext>,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        // The most specific provided target supplies the context, so a menu
        // or key invocation describes what it targets, not what has focus.
        let mut context = provided
            .as_ref()
            .and_then(|provided| {
                provided
                    .focused_pane_id
                    .as_deref()
                    .and_then(|pane_id| {
                        self.plugin_context_for_public_pane_id(pane_id, correlation_id)
                    })
                    .or_else(|| {
                        provided.tab_id.as_deref().and_then(|tab_id| {
                            self.plugin_context_for_tab_id(tab_id, correlation_id)
                        })
                    })
                    .or_else(|| {
                        provided.workspace_id.as_deref().and_then(|workspace_id| {
                            self.plugin_context_for_workspace_id(workspace_id, correlation_id)
                        })
                    })
            })
            .unwrap_or_else(|| self.current_plugin_context(correlation_id));
        if let Some(provided) = provided {
            context.workspace_id = provided.workspace_id.or(context.workspace_id);
            context.workspace_label = provided.workspace_label.or(context.workspace_label);
            context.workspace_cwd = provided.workspace_cwd.or(context.workspace_cwd);
            context.worktree = provided.worktree.or(context.worktree);
            context.tab_id = provided.tab_id.or(context.tab_id);
            context.tab_label = provided.tab_label.or(context.tab_label);
            context.focused_pane_id = provided.focused_pane_id.or(context.focused_pane_id);
            context.focused_pane_cwd = provided.focused_pane_cwd.or(context.focused_pane_cwd);
            context.focused_pane_agent = provided.focused_pane_agent.or(context.focused_pane_agent);
            context.focused_pane_status =
                provided.focused_pane_status.or(context.focused_pane_status);
            context.focused_pane_foreground_cwd = provided
                .focused_pane_foreground_cwd
                .or(context.focused_pane_foreground_cwd);
            context.focused_pane_agent_session = provided
                .focused_pane_agent_session
                .or(context.focused_pane_agent_session);
            context.selected_text = provided.selected_text.or(context.selected_text);
            context.invocation_source = provided.invocation_source.or(context.invocation_source);
            context.correlation_id = provided.correlation_id.or(context.correlation_id);
            context.clicked_url = provided.clicked_url.or(context.clicked_url);
            context.link_handler_id = provided.link_handler_id.or(context.link_handler_id);
            context.client_id = provided.client_id.or(context.client_id);
        }
        context
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

    pub(super) fn current_plugin_context(&self, correlation_id: &str) -> PluginInvocationContext {
        let Some(ws_idx) = self.state.active else {
            return empty_plugin_context(correlation_id);
        };
        self.plugin_context_for_workspace(ws_idx, correlation_id)
    }

    pub(super) fn plugin_context_for_event(
        &self,
        event: &crate::api::schema::EventEnvelope,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        match &event.data {
            EventData::WorkspaceCreated { workspace }
            | EventData::WorkspaceUpdated { workspace }
            | EventData::WorkspaceMetadataUpdated { workspace }
            | EventData::WorktreeCreated { workspace, .. }
            | EventData::WorktreeOpened { workspace, .. } => {
                self.plugin_context_for_workspace_info(workspace, correlation_id)
            }
            EventData::WorkspaceClosed {
                workspace_id,
                workspace,
            } => workspace
                .as_ref()
                .map(|workspace| self.plugin_context_for_workspace_info(workspace, correlation_id))
                .unwrap_or_else(|| {
                    self.plugin_context_for_workspace_id(workspace_id, correlation_id)
                        .unwrap_or_else(|| {
                            let mut context = empty_plugin_context(correlation_id);
                            context.workspace_id = Some(workspace_id.clone());
                            context
                        })
                }),
            EventData::WorkspaceReordered { workspace_ids, .. } => workspace_ids
                .first()
                .and_then(|workspace_id| {
                    self.plugin_context_for_workspace_id(workspace_id, correlation_id)
                })
                .unwrap_or_else(|| empty_plugin_context(correlation_id)),
            EventData::WorkspaceRenamed { workspace_id, .. }
            | EventData::WorkspaceMoved { workspace_id, .. }
            | EventData::WorkspaceFocused { workspace_id } => self
                .plugin_context_for_workspace_id(workspace_id, correlation_id)
                .unwrap_or_else(|| {
                    let mut context = empty_plugin_context(correlation_id);
                    context.workspace_id = Some(workspace_id.clone());
                    context
                }),
            EventData::WorktreeRemoved {
                workspace_id,
                workspace,
                worktree,
                ..
            } => workspace
                .as_ref()
                .map(|workspace| {
                    self.plugin_context_for_workspace_snapshot(workspace, correlation_id)
                })
                .or_else(|| self.plugin_context_for_workspace_id(workspace_id, correlation_id))
                .unwrap_or_else(|| {
                    let mut context = empty_plugin_context(correlation_id);
                    context.workspace_id = Some(workspace_id.clone());
                    context.workspace_label = Some(worktree.label.clone());
                    context.workspace_cwd = Some(worktree.path.clone());
                    context
                }),
            EventData::TabCreated { tab } => self.plugin_context_for_tab_info(tab, correlation_id),
            EventData::TabClosed {
                tab_id,
                workspace_id,
            } => {
                let mut context = empty_plugin_context(correlation_id);
                context.workspace_id = Some(workspace_id.clone());
                context.tab_id = Some(tab_id.clone());
                context
            }
            EventData::TabRenamed {
                tab_id,
                workspace_id,
                ..
            }
            | EventData::TabMoved {
                tab_id,
                workspace_id,
                ..
            }
            | EventData::TabFocused {
                tab_id,
                workspace_id,
            } => self
                .plugin_context_for_tab_id(tab_id, correlation_id)
                .or_else(|| self.plugin_context_for_workspace_id(workspace_id, correlation_id))
                .unwrap_or_else(|| {
                    let mut context = empty_plugin_context(correlation_id);
                    context.workspace_id = Some(workspace_id.clone());
                    context.tab_id = Some(tab_id.clone());
                    context
                }),
            EventData::LayoutUpdated { layout } => self
                .plugin_context_for_tab_id(&layout.tab_id, correlation_id)
                .or_else(|| {
                    self.plugin_context_for_workspace_id(&layout.workspace_id, correlation_id)
                })
                .unwrap_or_else(|| {
                    let mut context = empty_plugin_context(correlation_id);
                    context.workspace_id = Some(layout.workspace_id.clone());
                    context.tab_id = Some(layout.tab_id.clone());
                    context
                }),
            EventData::PaneCreated { pane } | EventData::PaneUpdated { pane } => {
                self.plugin_context_for_pane_info(pane, correlation_id)
            }
            EventData::PaneMoved { pane, .. } => {
                self.plugin_context_for_pane_info(pane.as_ref(), correlation_id)
            }
            EventData::PaneClosed {
                pane_id,
                workspace_id,
            } => {
                let mut context = empty_plugin_context(correlation_id);
                context.workspace_id = Some(workspace_id.clone());
                context.focused_pane_id = Some(pane_id.clone());
                context
            }
            EventData::PaneFocused {
                pane_id,
                workspace_id,
            }
            | EventData::PaneOutputChanged {
                pane_id,
                workspace_id,
                ..
            }
            | EventData::PaneExited {
                pane_id,
                workspace_id,
            }
            | EventData::PaneAgentDetected {
                pane_id,
                workspace_id,
                ..
            }
            | EventData::PaneAgentStatusChanged {
                pane_id,
                workspace_id,
                ..
            } => self
                .plugin_context_for_public_pane_id(pane_id, correlation_id)
                .or_else(|| self.plugin_context_for_workspace_id(workspace_id, correlation_id))
                .unwrap_or_else(|| {
                    let mut context = empty_plugin_context(correlation_id);
                    context.workspace_id = Some(workspace_id.clone());
                    context.focused_pane_id = Some(pane_id.clone());
                    context
                }),
        }
    }

    fn plugin_context_for_workspace_id(
        &self,
        workspace_id: &str,
        correlation_id: &str,
    ) -> Option<PluginInvocationContext> {
        let ws_idx = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .find_map(|(idx, _)| (self.public_workspace_id(idx) == workspace_id).then_some(idx))?;
        Some(self.plugin_context_for_workspace(ws_idx, correlation_id))
    }

    fn plugin_context_for_workspace_info(
        &self,
        workspace: &crate::api::schema::WorkspaceInfo,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        self.plugin_context_for_workspace_id(&workspace.workspace_id, correlation_id)
            .unwrap_or_else(|| {
                self.plugin_context_for_workspace_snapshot(workspace, correlation_id)
            })
    }

    fn plugin_context_for_workspace_snapshot(
        &self,
        workspace: &crate::api::schema::WorkspaceInfo,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        let mut context = empty_plugin_context(correlation_id);
        context.workspace_id = Some(workspace.workspace_id.clone());
        context.workspace_label = Some(workspace.label.clone());
        context.workspace_cwd = workspace
            .worktree
            .as_ref()
            .map(|worktree| worktree.checkout_path.clone());
        context.worktree = workspace.worktree.clone();
        context.tab_id = Some(workspace.active_tab_id.clone());
        context
    }

    fn plugin_context_for_tab_id(
        &self,
        tab_id: &str,
        correlation_id: &str,
    ) -> Option<PluginInvocationContext> {
        let (ws_idx, tab_idx) = self.parse_tab_id(tab_id)?;
        let ws = self.state.workspaces.get(ws_idx)?;
        let workspace = self.workspace_info(ws_idx);
        let tab = ws.tabs.get(tab_idx)?;
        let pane_id = tab.layout.focused();
        let focused_pane = self.pane_info(ws_idx, pane_id);
        Some(self.plugin_context_from_parts(
            ws_idx,
            workspace,
            self.public_tab_id(ws_idx, tab_idx),
            ws.tab_display_name(tab_idx),
            focused_pane,
            correlation_id,
        ))
    }

    fn plugin_context_for_tab_info(
        &self,
        tab: &crate::api::schema::TabInfo,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        self.plugin_context_for_tab_id(&tab.tab_id, correlation_id)
            .or_else(|| self.plugin_context_for_workspace_id(&tab.workspace_id, correlation_id))
            .unwrap_or_else(|| {
                let mut context = empty_plugin_context(correlation_id);
                context.workspace_id = Some(tab.workspace_id.clone());
                context.tab_id = Some(tab.tab_id.clone());
                context.tab_label = Some(tab.label.clone());
                context
            })
    }

    fn plugin_context_for_public_pane_id(
        &self,
        pane_id: &str,
        correlation_id: &str,
    ) -> Option<PluginInvocationContext> {
        let (ws_idx, pane_id) = self.parse_pane_id(pane_id)?;
        Some(self.plugin_context_for_pane(ws_idx, pane_id, correlation_id))
    }

    fn plugin_context_for_pane_info(
        &self,
        pane: &crate::api::schema::PaneInfo,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        self.plugin_context_for_public_pane_id(&pane.pane_id, correlation_id)
            .or_else(|| self.plugin_context_for_workspace_id(&pane.workspace_id, correlation_id))
            .unwrap_or_else(|| {
                let mut context = empty_plugin_context(correlation_id);
                context.workspace_id = Some(pane.workspace_id.clone());
                context.tab_id = Some(pane.tab_id.clone());
                context.focused_pane_id = Some(pane.pane_id.clone());
                context.focused_pane_cwd = pane.cwd.clone();
                context.focused_pane_agent = pane.agent.clone();
                context.focused_pane_status = Some(pane.agent_status);
                context.focused_pane_foreground_cwd = pane.foreground_cwd.clone();
                context.focused_pane_agent_session = pane.agent_session.clone();
                context
            })
    }

    pub(super) fn plugin_context_for_workspace(
        &self,
        ws_idx: usize,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        let Some(ws) = self.state.workspaces.get(ws_idx) else {
            return empty_plugin_context(correlation_id);
        };
        let workspace = self.workspace_info(ws_idx);
        let tab_idx = ws.active_tab_index();
        let tab_id = self.public_tab_id(ws_idx, tab_idx);
        let tab_label = ws.tab_display_name(tab_idx);
        let focused_pane = ws
            .focused_pane_id()
            .and_then(|pane_id| self.pane_info(ws_idx, pane_id));
        self.plugin_context_from_parts(
            ws_idx,
            workspace,
            tab_id,
            tab_label,
            focused_pane,
            correlation_id,
        )
    }

    pub(super) fn plugin_context_for_pane(
        &self,
        ws_idx: usize,
        pane_id: crate::layout::PaneId,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        let ws = &self.state.workspaces[ws_idx];
        let workspace = self.workspace_info(ws_idx);
        let tab_idx = ws
            .find_tab_index_for_pane(pane_id)
            .unwrap_or_else(|| ws.active_tab_index());
        let tab_id = self.public_tab_id(ws_idx, tab_idx);
        let tab_label = ws.tab_display_name(tab_idx);
        let focused_pane = self.pane_info(ws_idx, pane_id);
        self.plugin_context_from_parts(
            ws_idx,
            workspace,
            tab_id,
            tab_label,
            focused_pane,
            correlation_id,
        )
    }

    fn plugin_context_from_parts(
        &self,
        ws_idx: usize,
        workspace: crate::api::schema::WorkspaceInfo,
        tab_id: Option<String>,
        tab_label: Option<String>,
        focused_pane: Option<crate::api::schema::PaneInfo>,
        correlation_id: &str,
    ) -> PluginInvocationContext {
        let workspace_cwd = focused_pane
            .as_ref()
            .and_then(|pane| pane.cwd.clone())
            .or_else(|| Some(self.default_cwd_for_workspace(ws_idx).display().to_string()));
        PluginInvocationContext {
            workspace_id: Some(workspace.workspace_id),
            workspace_label: Some(workspace.label),
            workspace_cwd,
            worktree: workspace.worktree,
            tab_id,
            tab_label,
            focused_pane_id: focused_pane.as_ref().map(|pane| pane.pane_id.clone()),
            focused_pane_cwd: focused_pane.as_ref().and_then(|pane| pane.cwd.clone()),
            focused_pane_agent: focused_pane.as_ref().and_then(|pane| pane.agent.clone()),
            focused_pane_status: focused_pane.as_ref().map(|pane| pane.agent_status),
            focused_pane_foreground_cwd: focused_pane
                .as_ref()
                .and_then(|pane| pane.foreground_cwd.clone()),
            focused_pane_agent_session: focused_pane.and_then(|pane| pane.agent_session),
            // Selection is client presentation state. Client keybindings provide
            // revision-validated coordinates; API callers can provide explicit context.
            invocation_source: Some("api".to_string()),
            correlation_id: Some(correlation_id.to_string()),
            ..Default::default()
        }
    }

    fn default_cwd_for_workspace(&self, ws_idx: usize) -> std::path::PathBuf {
        self.state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| {
                ws.resolved_identity_cwd_from(&self.state.terminals, &self.terminal_runtimes)
            })
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()))
    }
}

fn empty_plugin_context(correlation_id: &str) -> PluginInvocationContext {
    PluginInvocationContext {
        invocation_source: Some("api".to_string()),
        correlation_id: Some(correlation_id.to_string()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::PluginInvocationContext;

    fn app_with_unfocused_target() -> crate::app::App {
        let mut app = super::super::tests::test_app();
        let mut target = crate::workspace::Workspace::test_new("target");
        target.custom_name = Some("Target".into());
        target.test_add_tab(Some("second"));
        app.state.workspaces = vec![crate::workspace::Workspace::test_new("focused"), target];
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        app
    }

    #[test]
    fn provided_targets_supply_context_most_specific_first() {
        let app = app_with_unfocused_target();
        let target_tab = app.public_tab_id(1, 1).unwrap();
        let target_pane = app
            .public_pane_id(1, app.state.workspaces[1].tabs[1].root_pane)
            .unwrap();
        let workspace_id = app.public_workspace_id(1);
        for provided in [
            PluginInvocationContext {
                focused_pane_id: Some(target_pane.clone()),
                ..Default::default()
            },
            PluginInvocationContext {
                tab_id: Some(target_tab.clone()),
                ..Default::default()
            },
        ] {
            let context = app.merge_plugin_context(Some(provided), "menu");
            assert_eq!(context.workspace_id.as_deref(), Some(workspace_id.as_str()));
            assert_eq!(context.workspace_label.as_deref(), Some("Target"));
            assert_eq!(context.tab_id.as_deref(), Some(target_tab.as_str()));
            assert_eq!(
                context.focused_pane_id.as_deref(),
                Some(target_pane.as_str())
            );
        }
    }

    #[test]
    fn invocation_details_carry_the_client_tabs_and_every_workspace() {
        let mut app = app_with_unfocused_target();
        app.invoking_client_id = Some(7);
        let context = app.with_invocation_details(PluginInvocationContext {
            workspace_id: Some(app.public_workspace_id(1)),
            ..Default::default()
        });
        assert_eq!(context.client_id, Some(7));
        assert_eq!(
            context
                .tabs
                .iter()
                .map(|tab| tab.tab_id.clone())
                .collect::<Vec<_>>(),
            vec![
                app.public_tab_id(1, 0).unwrap(),
                app.public_tab_id(1, 1).unwrap()
            ]
        );
        assert_eq!(
            context
                .workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone())
                .collect::<Vec<_>>(),
            vec![app.public_workspace_id(0), app.public_workspace_id(1)]
        );
    }

    #[test]
    fn event_contexts_stay_lean() {
        let app = app_with_unfocused_target();
        let context = app.plugin_context_for_event(
            &crate::api::schema::EventEnvelope {
                event: crate::api::schema::EventKind::WorkspaceFocused,
                data: crate::api::schema::EventData::WorkspaceFocused {
                    workspace_id: app.public_workspace_id(1),
                },
            },
            "workspace.focused",
        );
        assert!(context.client_id.is_none());
        assert!(context.tabs.is_empty());
        assert!(context.workspaces.is_empty());
    }
}
