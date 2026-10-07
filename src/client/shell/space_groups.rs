//! User-defined space groups in the spaces sidebar.
//!
//! A space joins a group through the `space_group` workspace metadata token,
//! which `workspace.set_group` sets. The sidebar never reorders spaces: it
//! shows the server order and starts a collapsible header wherever the group
//! changes, with a divider where ungrouped spaces follow a group. Worktree
//! families take their parent checkout's group (`crate::space_order`).
//! Spaces and headers drag anywhere, like in a flat sidebar, and the server
//! regroups what moved by where it landed. A header moves its whole run, so
//! dropping one inside another group splits that group into two runs.
//! Runs of one group share a collapse key, so collapsing either collapses both.
//!
//! Configured pinned sections (`[[ui.sidebar.spaces.pinned]]`) come first:
//! each shows a header and a flat mirror row for every space carrying its
//! token, in server order. Mirrors are copies; every space keeps its place in
//! the regular list, which follows behind a divider. Mirrors and pinned
//! headers neither drag nor take drops, and navigation skips mirrors.

use super::*;

/// Collapse state key of a group, namespaced apart from worktree repo keys.
fn collapse_key(name: &str) -> String {
    format!("group:{name}")
}

/// Collapse state key of a pinned section, namespaced apart from group and
/// worktree repo keys.
fn pinned_collapse_key(title: &str) -> String {
    format!("pinned:{title}")
}

/// A group header, or a pinned section's header.
pub(super) struct Group {
    pub(super) name: String,
    pub(super) collapsed: bool,
    pub(super) pinned: bool,
    /// Header text color; `None` keeps the default.
    pub(super) title_color: Option<ratatui::style::Color>,
}

impl Group {
    fn key(&self) -> String {
        if self.pinned {
            pinned_collapse_key(&self.name)
        } else {
            collapse_key(&self.name)
        }
    }
}

/// Starts a run: a group header, or with `group: None` the divider before
/// ungrouped spaces that follow a group.
pub(super) struct Marker {
    pub(super) group: Option<Group>,
    /// Every member's workspace index in expanded order, including members
    /// hidden by a collapsed group or worktree family.
    pub(super) members: Vec<usize>,
}

pub(super) enum SidebarRow {
    Marker(Marker),
    Workspace(WorkspaceEntry),
    /// A pinned section's copy of a space that also has its own row.
    Mirror(WorkspaceEntry),
}

pub(super) struct MarkerHit {
    pub(super) rect: Rect,
    pub(super) endpoint_id: ClientEndpointId,
    /// Collapse key of a group or pinned header; `None` for a divider.
    pub(super) key: Option<String>,
    /// A pinned section's header, which neither drags nor takes drops.
    pub(super) pinned: bool,
    /// The run's spaces in sidebar order, moved as one block when dragged.
    pub(super) member_ids: Vec<String>,
}

/// Blank rows between two adjacent sidebar rows: a marker stays attached to
/// the space below it, and worktree children to their parent.
pub(super) fn gap_between(previous: &SidebarRow, next: &SidebarRow, row_gap: u16) -> u16 {
    match (previous, next) {
        (SidebarRow::Marker(_), _) => 0,
        (_, SidebarRow::Workspace(entry)) if entry.indented => 0,
        _ => row_gap,
    }
}

/// The snapshot's spaces as `crate::space_order` sees them.
fn order_spaces(snapshot: &ClientShellSnapshot) -> Vec<crate::space_order::Space<'_>> {
    snapshot
        .workspaces
        .iter()
        .map(|workspace| crate::space_order::Space {
            worktree: workspace
                .worktree
                .as_ref()
                .map(|worktree| (worktree.key.as_str(), worktree.is_linked_worktree)),
            group: workspace
                .tokens
                .iter()
                .find(|(key, _)| key == crate::space_order::GROUP_TOKEN)
                .map(|(_, name)| name.as_str()),
        })
        .collect()
}

/// Splits sidebar entries into top-level units: an entry plus its indented children.
fn units(entries: Vec<WorkspaceEntry>) -> Vec<Vec<WorkspaceEntry>> {
    let mut units: Vec<Vec<WorkspaceEntry>> = Vec::new();
    for entry in entries {
        match units.last_mut() {
            Some(unit) if entry.indented => unit.push(entry),
            _ => units.push(vec![entry]),
        }
    }
    units
}

/// Consecutive units that share a group, or share having none.
struct Run<'a> {
    name: Option<&'a str>,
    members: Vec<usize>,
}

fn runs(snapshot: &ClientShellSnapshot) -> Vec<Run<'_>> {
    let spaces = order_spaces(snapshot);
    let mut runs = Vec::<Run<'_>>::new();
    for unit in units(sidebar::worktree_entries(snapshot, &HashSet::new())) {
        let name = crate::space_order::group(&spaces, unit[0].index);
        let members = unit.iter().map(|entry| entry.index);
        match runs.last_mut() {
            Some(run) if run.name == name => run.members.extend(members),
            _ => runs.push(Run {
                name,
                members: members.collect(),
            }),
        }
    }
    runs
}

/// Pinned sections with at least one matching space, in config order: a
/// header, then unless collapsed a flat mirror row per member.
fn pinned_rows(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
    pinned: &[crate::config::PinnedSpaceSection],
) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    for section in pinned {
        let members = snapshot
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, workspace)| {
                section.matches(
                    workspace
                        .tokens
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if members.is_empty() {
            continue;
        }
        let collapsed = collapsed_groups.contains(&pinned_collapse_key(&section.title));
        let mirrors = if collapsed {
            Vec::new()
        } else {
            members.clone()
        };
        rows.push(SidebarRow::Marker(Marker {
            group: Some(Group {
                name: section.title.clone(),
                collapsed,
                pinned: true,
                title_color: section.fg.map(|color| color.ratatui()),
            }),
            members,
        }));
        rows.extend(mirrors.into_iter().map(|index| {
            SidebarRow::Mirror(WorkspaceEntry {
                index,
                indented: false,
                last_child: false,
            })
        }));
    }
    rows
}

/// Sidebar rows for one endpoint: the pinned sections, then its spaces in
/// sidebar order with a marker starting each run after the first ungrouped
/// one, or every run when pinned sections precede the list. Without groups
/// and pinned sections this is upstream's flat list.
pub(super) fn sidebar_rows(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
    pinned: &[crate::config::PinnedSpaceSection],
) -> Vec<SidebarRow> {
    let mut rows = pinned_rows(snapshot, collapsed_groups, pinned);
    let after_pinned = !rows.is_empty();
    let runs = runs(snapshot);
    let mut run_of = vec![0; snapshot.workspaces.len()];
    for (index, run) in runs.iter().enumerate() {
        for member in &run.members {
            run_of[*member] = index;
        }
    }
    let mut shown = runs.iter().map(|_| Vec::new()).collect::<Vec<_>>();
    for entry in sidebar::worktree_entries(snapshot, collapsed_groups) {
        shown[run_of[entry.index]].push(entry);
    }

    for (index, (run, entries)) in runs.into_iter().zip(shown).enumerate() {
        let collapsed = run
            .name
            .is_some_and(|name| collapsed_groups.contains(&collapse_key(name)));
        if run.name.is_some() || index > 0 || after_pinned {
            rows.push(SidebarRow::Marker(Marker {
                group: run.name.map(|name| Group {
                    name: name.to_owned(),
                    collapsed,
                    pinned: false,
                    title_color: None,
                }),
                members: run.members,
            }));
        }
        if collapsed {
            // A collapsed group still shows its focused space, as a top-level row.
            rows.extend(
                entries
                    .into_iter()
                    .find(|entry| snapshot.workspaces[entry.index].focused)
                    .map(|entry| {
                        SidebarRow::Workspace(WorkspaceEntry {
                            indented: false,
                            last_child: false,
                            ..entry
                        })
                    }),
            );
        } else {
            rows.extend(entries.into_iter().map(SidebarRow::Workspace));
        }
    }
    rows
}

/// Navigable workspace entries in sidebar order. Pinned mirrors are left
/// out, so every space appears once.
pub(super) fn workspace_entries(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    sidebar_rows(snapshot, collapsed_groups, &[])
        .into_iter()
        .filter_map(|row| match row {
            SidebarRow::Workspace(entry) => Some(entry),
            SidebarRow::Marker(_) | SidebarRow::Mirror(_) => None,
        })
        .collect()
}

/// Draws a group header or the divider and records its hit.
pub(super) fn render_marker(
    buffer: &mut Buffer,
    rect: Rect,
    marker: &Marker,
    snapshot: &ClientShellSnapshot,
    endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    match &marker.group {
        Some(group) => render_group_header(
            buffer,
            rect,
            snapshot,
            group,
            &marker.members,
            config.status_indicators,
            &config.palette,
        ),
        None => {
            let width = rect.width.saturating_sub(2);
            super::render::put_text(
                buffer,
                rect.x.saturating_add(1),
                rect.y,
                width,
                &"─".repeat(width as usize),
                Style::default().fg(config.palette.surface_dim),
            );
        }
    }
    hits.markers.push(MarkerHit {
        rect,
        endpoint_id: endpoint_id.clone(),
        key: marker.group.as_ref().map(Group::key),
        pinned: marker.group.as_ref().is_some_and(|group| group.pinned),
        member_ids: marker
            .members
            .iter()
            .map(|index| snapshot.workspaces[*index].workspace_id.clone())
            .collect(),
    });
}

fn render_group_header(
    buffer: &mut Buffer,
    rect: Rect,
    snapshot: &ClientShellSnapshot,
    group: &Group,
    members: &[usize],
    indicators: crate::config::StatusIndicatorStyle,
    palette: &Palette,
) {
    if rect.is_empty() {
        return;
    }
    let right = rect.right().saturating_sub(1);
    let marker = if group.collapsed { "▸" } else { "▾" };
    let mut x = super::render::put_segment(
        buffer,
        rect.x.saturating_add(1),
        rect.y,
        right,
        marker,
        Style::default().fg(palette.accent),
    );
    x = super::render::put_segment(
        buffer,
        x.saturating_add(1),
        rect.y,
        right,
        &group.name,
        Style::default()
            .fg(group.title_color.unwrap_or(palette.subtext0))
            .add_modifier(Modifier::BOLD),
    );
    let count = members.len().to_string();
    let status = members
        .iter()
        .map(|index| snapshot.workspaces[*index].agent_status)
        .max_by_key(|status| status_priority(*status))
        .unwrap_or(crate::api::schema::AgentStatus::Unknown);
    let summary_width = if group.collapsed {
        UnicodeWidthStr::width(status_icon(status, indicators)) as u16 + 1
    } else {
        0
    } + count.len() as u16;
    let summary_x = right.saturating_sub(summary_width);
    if summary_x <= x.saturating_add(1) {
        return;
    }
    let rule_width = summary_x.saturating_sub(x.saturating_add(2));
    super::render::put_text(
        buffer,
        x.saturating_add(1),
        rect.y,
        rule_width,
        &"─".repeat(rule_width as usize),
        Style::default().fg(palette.surface_dim),
    );
    let mut summary = summary_x;
    if group.collapsed {
        summary = super::render::put_segment(
            buffer,
            summary,
            rect.y,
            right,
            status_icon(status, indicators),
            Style::default().fg(status_color(status, palette)),
        )
        .saturating_add(1);
    }
    super::render::put_segment(
        buffer,
        summary,
        rect.y,
        right,
        &count,
        Style::default().fg(palette.overlay0),
    );
}
