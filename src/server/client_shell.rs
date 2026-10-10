use ratatui::layout::Rect;

use crate::app;
use crate::protocol::{self, FrameData};

pub(super) fn snapshot(
    app: &app::App,
    boot_id: &str,
    revision: u64,
    config_diagnostic: Option<&str>,
    location: Option<&crate::server::clients::ClientShellLocation>,
) -> protocol::ClientShellSnapshot {
    let snapshot = app.session_metadata_snapshot();
    let focused_workspace_id = location
        .and_then(|location| location.focused_workspace_id.clone())
        .or_else(|| snapshot.focused_workspace_id.clone());
    let focused_tab_id = location
        .and_then(|location| location.focused_tab_id().map(str::to_owned))
        .or_else(|| snapshot.focused_tab_id.clone());
    let focused_pane_id = focused_tab_id
        .as_deref()
        .and_then(|tab_id| app.parse_tab_id(tab_id))
        .and_then(|(workspace_index, tab_index)| {
            let pane_id = app
                .state
                .workspaces
                .get(workspace_index)?
                .tabs
                .get(tab_index)?
                .layout
                .focused();
            app.public_pane_id(workspace_index, pane_id)
        })
        .or_else(|| snapshot.focused_pane_id.clone());
    let workspaces = snapshot
        .workspaces
        .into_iter()
        .zip(&app.state.workspaces)
        .enumerate()
        .map(|(workspace_index, (workspace, state))| {
            let mut tokens = workspace.tokens.into_iter().collect::<Vec<_>>();
            tokens.sort_by(|left, right| left.0.cmp(&right.0));
            let workspace_id = workspace.workspace_id;
            let active_tab_id = location
                .and_then(|location| location.active_tab_ids.get(&workspace_id))
                .cloned()
                .unwrap_or(workspace.active_tab_id);
            let active_tab_index =
                app.parse_tab_id(&active_tab_id)
                    .and_then(|(tab_workspace_index, tab_index)| {
                        (tab_workspace_index == workspace_index).then_some(tab_index)
                    });
            protocol::ClientShellWorkspace {
                focused: focused_workspace_id.as_deref() == Some(workspace_id.as_str()),
                workspace_id,
                active_tab_id,
                new_workspace_cwd: app
                    .resolved_new_workspace_cwd_from_tab(workspace_index, active_tab_index)
                    .display()
                    .to_string(),
                number: workspace.number,
                label: workspace.label,
                custom_label: state.custom_name.is_some(),
                branch: state.branch(),
                git_ahead_behind: state.git_ahead_behind(),
                tokens,
                worktree: workspace
                    .worktree
                    .map(|worktree| protocol::ClientShellWorktree {
                        key: worktree.repo_key,
                        label: worktree.repo_name,
                        is_linked_worktree: worktree.is_linked_worktree,
                    }),
                agent_status: workspace.agent_status,
            }
        })
        .collect::<Vec<protocol::ClientShellWorkspace>>();
    let pinned_sections = pinned_sections(&app.state.sidebar_spaces.pinned, &workspaces);
    let priority_marks = workspaces
        .iter()
        .map(|workspace| {
            (
                workspace.workspace_id.as_str(),
                crate::agent_priority::workspace_mark(&app.state.agent_priority_tokens, workspace),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let tabs = snapshot
        .tabs
        .into_iter()
        .zip(
            app.state
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.tabs.iter()),
        )
        .map(|(tab, state)| {
            let tab_id = tab.tab_id;
            protocol::ClientShellTab {
                focused: focused_tab_id.as_deref() == Some(tab_id.as_str()),
                tab_id,
                workspace_id: tab.workspace_id,
                number: tab.number,
                label: tab.label,
                custom_label: !state.is_auto_named(),
                zoomed: state.zoomed,
                agent_status: tab.agent_status,
            }
        })
        .collect();
    let panes = snapshot
        .panes
        .into_iter()
        .map(|pane| {
            let pane_id = pane.pane_id;
            let focused = focused_pane_id.as_deref() == Some(pane_id.as_str());
            let right_click_passthrough = app
                .parse_pane_id(&pane_id)
                .and_then(|(workspace_index, pane_id)| {
                    app.state
                        .workspaces
                        .get(workspace_index)?
                        .pane_state(pane_id)
                })
                .is_some_and(|pane| pane.right_click_passthrough);
            protocol::ClientShellPane {
                pane_id,
                workspace_id: pane.workspace_id,
                tab_id: pane.tab_id,
                label: pane.label,
                cwd: pane.cwd,
                foreground_cwd: pane.foreground_cwd,
                focused,
                right_click_passthrough,
            }
        })
        .collect();
    let agents = snapshot
        .agents
        .into_iter()
        .map(|agent| {
            let pane_id = agent.pane_id;
            let focused = focused_pane_id.as_deref() == Some(pane_id.as_str());
            let order_stamp = app
                .state
                .terminals
                .get(agent.terminal_id.as_str())
                .and_then(|terminal| terminal.agent_order_stamp)
                .unwrap_or(0);
            let mut state_labels = agent.state_labels.into_iter().collect::<Vec<_>>();
            state_labels.sort_by(|left, right| left.0.cmp(&right.0));
            let mut tokens = agent.tokens.into_iter().collect::<Vec<_>>();
            tokens.sort_by(|left, right| left.0.cmp(&right.0));
            let priority_mark = priority_marks
                .get(agent.workspace_id.as_str())
                .copied()
                .flatten();
            let priority_key = crate::agent_priority::priority_key(
                priority_mark.is_some(),
                agent.agent_status,
                order_stamp,
            );
            protocol::ClientShellAgent {
                pane_id,
                workspace_id: agent.workspace_id,
                tab_id: agent.tab_id,
                name: agent.name,
                display_agent: agent.display_agent,
                agent: agent.agent,
                title: agent.title,
                terminal_title: agent.terminal_title,
                terminal_title_stripped: agent.terminal_title_stripped,
                agent_status: agent.agent_status,
                state_change_seq: agent.state_change_seq,
                state_labels,
                tokens,
                focused,
                priority_key,
                priority_mark: priority_mark.map(str::to_owned),
            }
        })
        .collect();

    let agent_view_label = app
        .state
        .agent_view_override
        .as_ref()
        .map(|view| view.label.clone().unwrap_or_else(|| "filtered".to_owned()));
    let agent_order = crate::ui::agent_panel_entries_from(&app.state, &app.terminal_runtimes)
        .into_iter()
        .filter_map(|entry| app.public_pane_id(entry.ws_idx, entry.pane_id))
        .collect();

    let zoomed = focused_tab_id
        .as_deref()
        .and_then(|tab_id| app.parse_tab_id(tab_id))
        .and_then(|(workspace_index, tab_index)| {
            app.state
                .workspaces
                .get(workspace_index)?
                .tabs
                .get(tab_index)
        })
        .is_some_and(|tab| tab.zoomed);
    let tab_bar_right = app
        .state
        .tab_bar_right
        .iter()
        .filter_map(|segment| match segment {
            crate::app::state::TabBarStatusSegment::Zoom if zoomed => {
                Some(protocol::ClientShellTabStatusSegment {
                    text: "ZOOM".to_owned(),
                    accent: true,
                })
            }
            crate::app::state::TabBarStatusSegment::Text(Some(text)) if !text.is_empty() => {
                Some(protocol::ClientShellTabStatusSegment {
                    text: text.clone(),
                    accent: false,
                })
            }
            crate::app::state::TabBarStatusSegment::Zoom
            | crate::app::state::TabBarStatusSegment::Text(_) => None,
        })
        .collect();

    let product_announcement = app.state.product_announcement.as_ref().map(|announcement| {
        protocol::ClientShellProductAnnouncement {
            version: announcement.version.clone(),
            id: announcement.id.clone(),
            title: announcement.title.clone(),
            body: announcement.body.clone(),
            preview: announcement.preview,
        }
    });
    let release_notes =
        app.state
            .latest_release_notes
            .as_ref()
            .map(|notes| protocol::ClientShellReleaseNotes {
                version: notes.version.clone(),
                body: notes.body.clone(),
                preview: notes.preview,
            });

    protocol::ClientShellSnapshot {
        boot_id: boot_id.to_owned(),
        revision,
        config_diagnostic: config_diagnostic.map(str::to_owned),
        product_announcement,
        update_available: app.state.update_available.clone(),
        update_install_command: app.state.update_install_command.clone(),
        server_keybindings_toml: app.client_shell_keybindings_profile().map(str::to_owned),
        latest_release_notes_available: app.state.latest_release_notes_available,
        integration_updates_available: app.state.integration_updates_available(),
        worktree_directory: app.state.worktree_directory.to_string_lossy().into_owned(),
        release_notes,
        focused_workspace_id,
        focused_tab_id,
        focused_pane_id,
        tab_bar_right,
        tab_bar_right_separator: app.state.tab_bar_right_separator.clone(),
        agent_view_label,
        agent_order,
        workspaces,
        tabs,
        panes,
        agents,
        commands: app.client_shell_command_manifest(),
        pinned_sections,
    }
}

/// The configured pinned sections that at least one space matches, each with
/// its members in `workspaces` order.
pub(crate) fn pinned_sections(
    configured: &[crate::config::PinnedSpaceSection],
    workspaces: &[protocol::ClientShellWorkspace],
) -> Vec<protocol::ClientShellPinnedSection> {
    configured
        .iter()
        .filter_map(|section| {
            let workspace_ids = workspaces
                .iter()
                .filter(|workspace| {
                    section.matches(
                        workspace
                            .tokens
                            .iter()
                            .map(|(key, value)| (key.as_str(), value.as_str())),
                    )
                })
                .map(|workspace| workspace.workspace_id.clone())
                .collect::<Vec<_>>();
            (!workspace_ids.is_empty()).then(|| protocol::ClientShellPinnedSection {
                title: section.title.clone(),
                fg: section.fg.map(|color| color.hex()),
                workspace_ids,
            })
        })
        .collect()
}

pub(super) struct RenderedPaneSurface {
    pub(super) frame: FrameData,
    pub(super) panes: Vec<protocol::PaneSurfacePane>,
    pub(super) splits: Vec<protocol::PaneSurfaceSplit>,
    pub(super) popup: Option<Box<protocol::ClientShellPopupSurface>>,
    pub(super) graphics: protocol::SurfaceGraphicsScene,
    pub(super) graphics_delivery: crate::kitty_graphics::surface::DeliveryCache,
    pub(super) graphics_sources: crate::kitty_graphics::surface::SourceFiles,
}

#[derive(Debug)]
pub(super) enum SurfaceRenderDeferred {
    Synchronized,
    Changed,
}

pub(super) fn render_pane_surface(
    app: &mut app::App,
    target: Option<crate::ui::TabSurfaceTarget>,
    area: Rect,
    resize_panes: bool,
    show_popup: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
    graphics_delivery: &crate::kitty_graphics::surface::DeliveryCache,
    client_id: u64,
) -> Result<RenderedPaneSurface, SurfaceRenderDeferred> {
    let layout = crate::ui::compute_tab_surface_for(
        &app.state,
        &app.terminal_runtimes,
        target,
        area,
        resize_panes,
        cell_size,
    );
    let mut content_revisions_before = std::collections::HashMap::new();
    if let Some(target) = target {
        for pane in &layout.pane_infos {
            if let Some(runtime) = app.state.runtime_for_pane_in_workspace(
                &app.terminal_runtimes,
                target.workspace_index,
                pane.id,
            ) {
                let (synchronized, epoch) = runtime.synchronized_output_state();
                if synchronized {
                    return Err(SurfaceRenderDeferred::Synchronized);
                }
                let revision = runtime.content_seq();
                content_revisions_before.insert(pane.id, (epoch, revision));
            }
        }
    }
    let popup_revision_before = if show_popup {
        app.state
            .popup_pane
            .as_ref()
            .and_then(|popup| app.terminal_runtimes.get(&popup.terminal_id))
            .map(|runtime| {
                let (synchronized, epoch) = runtime.synchronized_output_state();
                if synchronized {
                    return Err(SurfaceRenderDeferred::Synchronized);
                }
                Ok(epoch)
            })
            .transpose()?
    } else {
        None
    };
    let (buffer, cursor, hyperlinks, layout) =
        crate::server::render_stream::render_tab_surface_virtual(
            &app.state,
            &app.terminal_runtimes,
            layout,
            area,
        );
    let panes = target
        .map(|target| {
            let workspace_index = target.workspace_index;
            layout
                .pane_infos
                .iter()
                .filter_map(|pane| {
                    app.public_pane_id(workspace_index, pane.id).map(|pane_id| {
                        let runtime = app.state.runtime_for_pane_in_workspace(
                            &app.terminal_runtimes,
                            workspace_index,
                            pane.id,
                        );
                        let mouse_reporting =
                            runtime.is_some_and(|runtime| runtime.mouse_reporting_enabled());
                        let sgr_pixel_mouse =
                            runtime.is_some_and(|runtime| runtime.sgr_pixel_mouse_enabled());
                        let (pixel_width, pixel_height) = if cell_size.is_known() {
                            (
                                u32::from(pane.inner_rect.width) * cell_size.width_px,
                                u32::from(pane.inner_rect.height) * cell_size.height_px,
                            )
                        } else {
                            (0, 0)
                        };
                        let content_revision = runtime.map_or(0, |runtime| {
                            let after = runtime.content_seq();
                            if content_revisions_before
                                .get(&pane.id)
                                .is_some_and(|&(_, before)| before == after)
                                && after.is_multiple_of(2)
                            {
                                after
                            } else {
                                after | 1
                            }
                        });
                        protocol::PaneSurfacePane {
                            pane_id,
                            content_revision,
                            rect: pane.rect.into(),
                            inner_rect: pane.inner_rect.into(),
                            scrollbar_rect: pane.scrollbar_rect.map(Into::into),
                            scroll: runtime.and_then(|runtime| runtime.scroll_metrics()).map(
                                |metrics| protocol::PaneSurfaceScrollMetrics {
                                    offset_from_bottom: metrics.offset_from_bottom as u64,
                                    max_offset_from_bottom: metrics.max_offset_from_bottom as u64,
                                    viewport_rows: metrics.viewport_rows as u64,
                                },
                            ),
                            focused: pane.is_focused,
                            mouse_reporting,
                            sgr_pixel_mouse,
                            alternate_screen_active: runtime
                                .is_some_and(|runtime| runtime.alternate_screen_active()),
                            pixel_width,
                            pixel_height,
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let pane_frames = layout
        .pane_infos
        .iter()
        .map(|pane| pane.rect)
        .collect::<Vec<_>>();
    let splits = layout
        .split_borders
        .iter()
        .filter_map(|split| {
            let hit_rect = split_hit_rect(
                split,
                app.state.pane_borders.draws_borders(),
                app.state.pane_gaps,
                &pane_frames,
            )?;
            let direction = match split.direction {
                ratatui::layout::Direction::Horizontal => {
                    protocol::PaneSurfaceSplitDirection::Horizontal
                }
                ratatui::layout::Direction::Vertical => {
                    protocol::PaneSurfaceSplitDirection::Vertical
                }
            };
            Some(protocol::PaneSurfaceSplit {
                direction,
                pos: split.pos,
                area: split.area.into(),
                hit_rect: hit_rect.into(),
                path: split.path.clone(),
            })
        })
        .collect();
    let popup = show_popup
        .then(|| render_popup_surface(app, area, resize_panes, cell_size))
        .flatten();
    let (graphics, next_graphics_delivery, graphics_sources) =
        crate::server::client_shell_graphics::collect(
            app,
            &layout.pane_infos,
            &layout.split_borders,
            popup.as_deref(),
            target,
            cell_size,
            graphics_delivery,
            client_id,
        );
    if let Some(target) = target {
        for (&pane_id, &(epoch, _)) in &content_revisions_before {
            if let Some(runtime) = app.state.runtime_for_pane_in_workspace(
                &app.terminal_runtimes,
                target.workspace_index,
                pane_id,
            ) {
                let (synchronized, after_epoch) = runtime.synchronized_output_state();
                if synchronized {
                    return Err(SurfaceRenderDeferred::Synchronized);
                }
                if after_epoch != epoch {
                    return Err(SurfaceRenderDeferred::Changed);
                }
            }
        }
    }
    if let Some(before) = popup_revision_before {
        if let Some(runtime) = app
            .state
            .popup_pane
            .as_ref()
            .and_then(|popup| app.terminal_runtimes.get(&popup.terminal_id))
        {
            let (synchronized, after_epoch) = runtime.synchronized_output_state();
            if synchronized {
                return Err(SurfaceRenderDeferred::Synchronized);
            }
            if after_epoch != before {
                return Err(SurfaceRenderDeferred::Changed);
            }
        }
    }
    Ok(RenderedPaneSurface {
        frame: FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, cursor, &hyperlinks),
        panes,
        splits,
        popup,
        graphics,
        graphics_delivery: next_graphics_delivery,
        graphics_sources,
    })
}

fn render_popup_surface(
    app: &app::App,
    area: Rect,
    resize_runtime: bool,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Option<Box<protocol::ClientShellPopupSurface>> {
    let popup = app.state.popup_pane.as_ref()?;
    let geometry = if resize_runtime {
        resize_popup_runtime(app, area, cell_size)?
    } else {
        crate::popup_size::resolve_popup_geometry(popup.width, popup.height, area)?
    };
    let runtime = app.terminal_runtimes.get(&popup.terminal_id)?;
    let content_area = Rect::new(0, 0, geometry.inner.width, geometry.inner.height);
    let (buffer, cursor) =
        crate::server::render_stream::render_terminal_virtual(runtime, content_area);
    let hyperlinks = runtime.visible_hyperlinks(content_area);
    let title = app
        .state
        .terminals
        .get(&popup.terminal_id)
        .and_then(|terminal| terminal.manual_label.clone())
        .unwrap_or_else(|| "popup".to_owned());
    let (pixel_width, pixel_height) = if cell_size.is_known() {
        (
            u32::from(content_area.width) * cell_size.width_px,
            u32::from(content_area.height) * cell_size.height_px,
        )
    } else {
        (0, 0)
    };
    Some(Box::new(protocol::ClientShellPopupSurface {
        terminal_id: popup.terminal_id.to_string(),
        title,
        width: popup.width.map(client_popup_size),
        height: popup.height.map(client_popup_size),
        frame: FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, cursor, &hyperlinks),
        mouse_reporting: runtime.mouse_reporting_enabled(),
        sgr_pixel_mouse: runtime.sgr_pixel_mouse_enabled(),
        pixel_width,
        pixel_height,
    }))
}

pub(super) fn resize_popup_runtime(
    app: &app::App,
    area: Rect,
    cell_size: crate::kitty_graphics::HostCellSize,
) -> Option<crate::popup_size::PopupResolvedGeometry> {
    let popup = app.state.popup_pane.as_ref()?;
    let geometry = crate::popup_size::resolve_popup_geometry(popup.width, popup.height, area)?;
    let runtime = app.terminal_runtimes.get(&popup.terminal_id)?;
    if !app
        .state
        .direct_attach_resize_locks
        .contains(&popup.terminal_id)
    {
        runtime.resize(
            geometry.inner.height,
            geometry.inner.width,
            cell_size.width_px,
            cell_size.height_px,
        );
    }
    Some(geometry)
}

fn client_popup_size(size: crate::popup_size::PopupSize) -> protocol::ClientShellPopupSize {
    match size {
        crate::popup_size::PopupSize::Cells(cells) => protocol::ClientShellPopupSize::Cells(cells),
        crate::popup_size::PopupSize::Percent(percent) => {
            protocol::ClientShellPopupSize::Percent(percent)
        }
    }
}

fn split_hit_rect(
    split: &crate::layout::SplitBorder,
    pane_borders: bool,
    pane_gaps: bool,
    pane_frames: &[Rect],
) -> Option<Rect> {
    let hit = match (split.direction, pane_borders, pane_gaps) {
        (ratatui::layout::Direction::Horizontal, true, false) => {
            Rect::new(split.pos, split.area.y, 1, split.area.height)
        }
        (ratatui::layout::Direction::Horizontal, true, true) => {
            let start = split.pos.saturating_sub(1);
            Rect::new(
                start,
                split.area.y,
                split.pos.saturating_sub(start).saturating_add(1),
                split.area.height,
            )
        }
        (ratatui::layout::Direction::Horizontal, false, true) => Rect::new(
            split.pos.checked_sub(1)?,
            split.area.y,
            1,
            split.area.height,
        ),
        (ratatui::layout::Direction::Vertical, true, false) => {
            Rect::new(split.area.x, split.pos, split.area.width, 1)
        }
        (ratatui::layout::Direction::Vertical, true, true) => {
            let start = split.pos.saturating_sub(1);
            Rect::new(
                split.area.x,
                start,
                split.area.width,
                split.pos.saturating_sub(start).saturating_add(1),
            )
        }
        (ratatui::layout::Direction::Vertical, false, true) => {
            Rect::new(split.area.x, split.pos.checked_sub(1)?, split.area.width, 1)
        }
        (_, false, false) => return None,
    };
    if !pane_borders
        && pane_frames.iter().any(|pane| {
            hit.x < pane.right()
                && hit.right() > pane.x
                && hit.y < pane.bottom()
                && hit.bottom() > pane.y
        })
    {
        return None;
    }
    Some(hit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn snapshot_metadata_skips_scroll_reads_and_preserves_public_session_data() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        let mut workspace = crate::workspace::Workspace::test_new("active");
        workspace.test_add_tab(None);
        app.state.workspaces = vec![workspace, crate::workspace::Workspace::test_new("hidden")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let history = (0..40)
            .map(|line| format!("line-{line}\r\n"))
            .collect::<String>();
        for (index, terminal) in app.state.terminals.values_mut().enumerate() {
            if index < 2 {
                terminal.set_detected_state(
                    Some(crate::detect::Agent::Pi),
                    crate::detect::AgentState::Working,
                );
                terminal.agent_name = Some(format!("agent-{index}"));
                terminal.last_agent_state_change_seq = Some(5);
                terminal.last_agent_completion_seq = Some(7);
            }
            let runtime = crate::terminal::TerminalRuntime::test_with_scrollback_bytes(
                20,
                5,
                1000,
                history.as_bytes(),
            );
            runtime.scroll_up(3);
            app.terminal_runtimes.insert(terminal.id.clone(), runtime);
        }

        let metadata = app.session_metadata_snapshot();
        assert_eq!(metadata.panes.len(), 3);
        assert_eq!(metadata.agents.len(), 2);
        assert!(metadata.panes.iter().all(|pane| pane.scroll.is_none()));
        assert!(app
            .terminal_runtimes
            .values()
            .all(|runtime| runtime.test_scroll_metrics_reads() == 0));

        let mut public = app.session_snapshot();
        for pane in &mut public.panes {
            let scroll = pane
                .scroll
                .take()
                .expect("public session must expose scroll");
            assert_eq!(scroll.offset_from_bottom, 3);
            assert!(scroll.max_offset_from_bottom >= 3);
            assert_eq!(scroll.viewport_rows, 5);
        }
        assert_eq!(metadata, public);
        assert!(app
            .terminal_runtimes
            .values()
            .all(|runtime| runtime.test_scroll_metrics_reads() == 1));

        let (shell, completions) = snapshot_with_completions(&app, "boot", 9, None, None);
        assert_eq!(shell.panes.len(), 3);
        assert_eq!(shell.agents.len(), 2);
        assert_eq!(shell.focused_pane_id, metadata.focused_pane_id);
        for (pane, public_pane) in shell.panes.iter().zip(&metadata.panes) {
            assert_eq!(pane.pane_id, public_pane.pane_id);
            assert_eq!(pane.workspace_id, public_pane.workspace_id);
            assert_eq!(pane.tab_id, public_pane.tab_id);
            assert_eq!(pane.cwd, public_pane.cwd);
        }
        for (agent, public_agent) in shell.agents.iter().zip(&metadata.agents) {
            assert_eq!(agent.pane_id, public_agent.pane_id);
            assert_eq!(agent.name, public_agent.name);
            assert_eq!(agent.agent_status, public_agent.agent_status);
            assert_eq!(agent.state_change_seq, 5);
        }
        assert_eq!(completions.revision, 9);
        assert_eq!(completions.completions.len(), 2);
        assert!(completions.completions.iter().all(|(_, seq)| *seq == 7));
        assert!(app
            .terminal_runtimes
            .values()
            .all(|runtime| runtime.test_scroll_metrics_reads() == 1));
    }

    #[test]
    fn snapshot_projects_cached_release_and_update_facts() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.integration_recommendations.clear();
        app.state.update_available = Some("0.8.3".into());
        app.state.update_install_command = "herdr update".into();
        app.state.latest_release_notes_available = true;
        app.state.latest_release_notes = Some(crate::release_notes::ReleaseNotes {
            version: "0.8.3".into(),
            body: "### Changed\n- Client shell".into(),
            preview: true,
        });

        let snapshot = snapshot(&app, "boot", 7, None, None);

        assert_eq!(snapshot.update_available.as_deref(), Some("0.8.3"));
        assert_eq!(snapshot.update_install_command, "herdr update");
        assert!(snapshot.latest_release_notes_available);
        assert!(!snapshot.integration_updates_available);
        assert_eq!(
            snapshot.release_notes.as_ref().map(|notes| (
                notes.version.as_str(),
                notes.body.as_str(),
                notes.preview
            )),
            Some(("0.8.3", "### Changed\n- Client shell", true))
        );
    }

    #[test]
    fn snapshot_badges_only_outdated_integrations() {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.integration_recommendations =
            vec![crate::integration::IntegrationRecommendation {
                target: crate::api::schema::IntegrationTarget::Claude,
                label: "claude",
                command: "claude",
                available: true,
                path: std::path::PathBuf::from("claude-hook"),
                state: crate::integration::IntegrationStatusKind::NotInstalled,
            }];

        assert!(!snapshot(&app, "boot", 1, None, None).integration_updates_available);

        app.state.integration_recommendations[0].state =
            crate::integration::IntegrationStatusKind::Outdated;
        assert!(snapshot(&app, "boot", 2, None, None).integration_updates_available);
    }

    /// One agent per named space, each in the given state and seen flag.
    fn app_with_agents(agents: &[(&str, crate::detect::AgentState, bool)]) -> crate::app::App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = agents
            .iter()
            .map(|(name, _, _)| crate::workspace::Workspace::test_new(name))
            .collect();
        app.state.ensure_test_terminals();
        app.state.active = Some(0);
        for (ws_idx, &(_, state, seen)) in agents.iter().enumerate() {
            let pane_id = app.state.workspaces[ws_idx].tabs[0].root_pane;
            let pane = app.state.workspaces[ws_idx]
                .pane_state_mut(pane_id)
                .unwrap();
            pane.seen = seen;
            let terminal_id = pane.attached_terminal_id.clone();
            let (sequence, stamp) = app.state.next_agent_order_event();
            let terminal = app.state.terminals.get_mut(&terminal_id).unwrap();
            terminal.detected_agent = Some(crate::detect::Agent::Claude);
            terminal.state = state;
            terminal.last_agent_state_change_seq = Some(sequence);
            terminal.agent_order_stamp = Some(stamp);
        }
        app
    }

    fn set_token(app: &mut crate::app::App, ws_idx: usize, key: &str, value: &str) {
        app.state.workspaces[ws_idx].metadata_tokens.patch(
            std::collections::HashMap::from([(key.to_owned(), Some(value.to_owned()))]),
            None,
            std::time::Instant::now(),
        );
    }

    fn pin(app: &mut crate::app::App, sections: &str) {
        app.state.sidebar_spaces =
            toml::from_str::<crate::config::SpacesSidebarConfig>(sections).unwrap();
    }

    #[test]
    fn snapshot_resolves_pinned_sections_in_config_and_space_order() {
        use crate::detect::AgentState::Idle;
        let mut app = app_with_agents(&[
            ("api", Idle, true),
            ("web", Idle, true),
            ("docs", Idle, true),
            ("infra", Idle, true),
        ]);
        set_token(&mut app, 0, "prio", "high");
        set_token(&mut app, 1, "prio", "");
        set_token(&mut app, 2, "prio", "low");
        set_token(&mut app, 3, "prio", "highest");
        set_token(&mut app, 3, "team", "platform-eu");
        set_token(&mut app, 1, "team", "web");
        pin(
            &mut app,
            r##"
            [[pinned]]
            title = "Any priority"
            token = "prio"
            fg = "#F80"

            [[pinned]]
            title = "Exactly high"
            token = "prio"
            equals = "high"

            [[pinned]]
            title = "Platform"
            token = "team"
            starts_with = "platform"
            fg = "#0a1b2c"

            [[pinned]]
            title = "Mentions eb"
            token = "team"
            contains = "eb"

            [[pinned]]
            title = "Nobody"
            token = "owner"

            [[pinned]]
            title = "No such value"
            token = "prio"
            equals = "urgent"
            "##,
        );

        let first = snapshot(&app, "boot", 1, None, None);

        let id = |index: usize| first.workspaces[index].workspace_id.clone();
        let section =
            |title: &str, fg: Option<&str>, members: &[usize]| protocol::ClientShellPinnedSection {
                title: title.into(),
                fg: fg.map(str::to_owned),
                workspace_ids: members.iter().map(|&index| id(index)).collect(),
            };
        assert_eq!(
            first.pinned_sections,
            [
                // An empty value pins nothing; members keep the space order.
                section("Any priority", Some("#ff8800"), &[0, 2, 3]),
                // `equals` is the whole value, not a prefix of "highest".
                section("Exactly high", None, &[0]),
                section("Platform", Some("#0a1b2c"), &[3]),
                section("Mentions eb", None, &[1]),
                // Sections no space matches are left out.
            ]
        );

        // Members follow the endpoint's space order when it changes.
        app.state.workspaces.swap(0, 3);
        let reordered = snapshot(&app, "boot", 2, None, None);
        assert_eq!(
            reordered.pinned_sections[0].workspace_ids,
            [id(3), id(2), id(0)]
        );
        assert_eq!(
            reordered
                .workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone())
                .collect::<Vec<_>>(),
            [id(3), id(1), id(2), id(0)]
        );

        pin(&mut app, "");
        assert!(snapshot(&app, "boot", 3, None, None)
            .pinned_sections
            .is_empty());
    }

    /// Pane ids in the order a client gets by sorting on `priority_key`,
    /// ties keeping snapshot order.
    fn key_order(snapshot: &protocol::ClientShellSnapshot) -> Vec<String> {
        let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
        agents.sort_by(|left, right| left.priority_key.cmp(&right.priority_key));
        agents
            .into_iter()
            .map(|agent| agent.pane_id.clone())
            .collect()
    }

    #[test]
    fn snapshot_priority_keys_reproduce_the_priority_order_with_the_lift() {
        use crate::detect::AgentState::{Blocked, Idle, Working};
        let mut app = app_with_agents(&[
            ("idle-old", Idle, true),
            ("done-old", Idle, false),
            ("working", Working, true),
            ("blocked", Blocked, true),
            ("done-new", Idle, false),
            ("idle-new", Idle, true),
        ]);
        app.state.agent_panel_sort = crate::app::state::AgentPanelSort::Priority;
        let pane = |snapshot: &protocol::ClientShellSnapshot, index: usize| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.workspace_id == snapshot.workspaces[index].workspace_id)
                .unwrap()
                .pane_id
                .clone()
        };

        let plain = snapshot(&app, "boot", 1, None, None);
        // Blocked, then done oldest first, then working, then idle newest first.
        let expected = [3, 1, 4, 2, 5, 0].map(|index| pane(&plain, index));
        assert_eq!(key_order(&plain), expected);
        assert_eq!(plain.agent_order, expected);
        assert!(plain
            .agents
            .iter()
            .all(|agent| agent.priority_mark.is_none() && agent.priority_key.starts_with("1.")));

        // The second listed token marks nothing while the first has a value.
        app.state.agent_priority_tokens = vec!["lift".into(), "urgent".into()];
        set_token(&mut app, 4, "urgent", "!");
        set_token(&mut app, 4, "lift", "★");
        set_token(&mut app, 2, "lift", "now");
        set_token(&mut app, 0, "lift", "");
        set_token(&mut app, 3, "unlisted", "x");
        let lifted = snapshot(&app, "boot", 2, None, None);
        // The lifted done agent passes the blocked one; the lifted working
        // agent keeps its place, and so does the space with an empty token.
        let expected = [4, 3, 1, 2, 5, 0].map(|index| pane(&lifted, index));
        assert_eq!(key_order(&lifted), expected);
        assert_eq!(lifted.agent_order, expected);
        let marks = lifted
            .workspaces
            .iter()
            .map(|workspace| {
                lifted
                    .agents
                    .iter()
                    .find(|agent| agent.workspace_id == workspace.workspace_id)
                    .unwrap()
                    .priority_mark
                    .as_deref()
            })
            .collect::<Vec<_>>();
        assert_eq!(marks, [None, None, Some("now"), None, Some("★"), None]);
    }

    #[test]
    fn snapshot_priority_keys_requeue_a_restored_completion() {
        use crate::detect::AgentState::Idle;
        let mut app = app_with_agents(&[
            ("held", Idle, true),
            ("done-first", Idle, false),
            ("done-second", Idle, false),
        ]);
        let order = |app: &crate::app::App| {
            let snapshot = snapshot(app, "boot", 1, None, None);
            key_order(&snapshot)
                .into_iter()
                .map(|pane_id| {
                    snapshot
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane_id)
                        .map(|agent| {
                            snapshot
                                .workspaces
                                .iter()
                                .position(|workspace| workspace.workspace_id == agent.workspace_id)
                                .unwrap()
                        })
                        .unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(order(&app), [1, 2, 0]);

        let held = app.state.workspaces[0].tabs[0].root_pane;
        app.state.arm_unread_hold(0, held, None).unwrap();
        assert_eq!(order(&app), [1, 2, 0]);
        // Although it changed first, it now waits behind both completions.
        assert!(app.restore_unread_hold(0, held));
        assert_eq!(order(&app), [1, 2, 0]);
        let restored = snapshot(&app, "boot", 2, None, None);
        assert_eq!(
            restored.agents[0].agent_status,
            crate::api::schema::AgentStatus::Done
        );

        // Seen again, it is the freshest idle agent.
        assert!(app.state.clear_unread_hold(0, held));
        let first_done = app.state.workspaces[1].tabs[0].root_pane;
        app.state.workspaces[1]
            .pane_state_mut(first_done)
            .unwrap()
            .seen = true;
        assert_eq!(order(&app), [2, 0, 1]);
    }

    #[test]
    fn split_hits_follow_released_border_and_gap_geometry() {
        let horizontal = crate::layout::SplitBorder {
            pos: 20,
            direction: ratatui::layout::Direction::Horizontal,
            ratio: 0.5,
            area: Rect::new(2, 3, 40, 12),
            path: vec![false],
        };
        assert_eq!(
            split_hit_rect(&horizontal, true, false, &[]),
            Some(Rect::new(20, 3, 1, 12))
        );
        assert_eq!(
            split_hit_rect(&horizontal, true, true, &[]),
            Some(Rect::new(19, 3, 2, 12))
        );
        assert_eq!(
            split_hit_rect(&horizontal, false, true, &[]),
            Some(Rect::new(19, 3, 1, 12))
        );
        assert_eq!(split_hit_rect(&horizontal, false, false, &[]), None);

        let vertical = crate::layout::SplitBorder {
            pos: 9,
            direction: ratatui::layout::Direction::Vertical,
            ratio: 0.5,
            area: Rect::new(2, 3, 40, 12),
            path: vec![true],
        };
        assert_eq!(
            split_hit_rect(&vertical, true, true, &[]),
            Some(Rect::new(2, 8, 40, 2))
        );

        let edge = crate::layout::SplitBorder {
            pos: 0,
            direction: ratatui::layout::Direction::Horizontal,
            ratio: 0.5,
            area: Rect::new(0, 0, 1, 4),
            path: Vec::new(),
        };
        assert_eq!(
            split_hit_rect(&edge, true, true, &[]),
            Some(Rect::new(0, 0, 1, 4))
        );
        assert_eq!(split_hit_rect(&edge, false, true, &[]), None);
        assert_eq!(
            split_hit_rect(&horizontal, false, true, &[Rect::new(19, 3, 1, 12)]),
            None
        );
    }
}
