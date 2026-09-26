//! User-defined space groups in the spaces sidebar.
//!
//! A space joins a group through the `space_group` workspace metadata token,
//! which the space-groups plugin sets. The sidebar never reorders spaces: it
//! shows the server order and starts a collapsible header wherever the group
//! changes, with a divider where ungrouped spaces follow a group. Worktree
//! families take their parent checkout's group. Membership is the plugin's
//! business: spaces and headers drag anywhere, like in a flat sidebar, and the
//! plugin regroups what moved by where it landed. A header moves its whole
//! run, so dropping one inside another group splits that group into two runs.
//! Runs of one group share a collapse key, so collapsing either collapses both.

use super::*;

/// Workspace metadata key that names a space's group.
const GROUP_TOKEN: &str = "space_group";

/// Collapse state key of a group, namespaced apart from worktree repo keys.
fn collapse_key(name: &str) -> String {
    format!("group:{name}")
}

pub(super) struct Group {
    pub(super) name: String,
    pub(super) collapsed: bool,
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
}

pub(super) struct MarkerHit {
    pub(super) rect: Rect,
    pub(super) endpoint_id: ClientEndpointId,
    /// Collapse key of a group header; `None` for a divider.
    pub(super) key: Option<String>,
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

fn workspace_group(workspace: &ClientShellWorkspace) -> Option<&str> {
    workspace
        .tokens
        .iter()
        .find(|(key, _)| key == GROUP_TOKEN)
        .map(|(_, name)| name.as_str())
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
    let mut runs = Vec::<Run<'_>>::new();
    for unit in units(sidebar::worktree_entries(snapshot, &HashSet::new())) {
        let name = workspace_group(&snapshot.workspaces[unit[0].index]);
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

/// Sidebar rows for one endpoint: its spaces in sidebar order, with a marker
/// starting each run after the first ungrouped one. Without groups this is
/// upstream's flat list.
pub(super) fn sidebar_rows(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<SidebarRow> {
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

    let mut rows = Vec::new();
    for (index, (run, entries)) in runs.into_iter().zip(shown).enumerate() {
        let collapsed = run
            .name
            .is_some_and(|name| collapsed_groups.contains(&collapse_key(name)));
        if run.name.is_some() || index > 0 {
            rows.push(SidebarRow::Marker(Marker {
                group: run.name.map(|name| Group {
                    name: name.to_owned(),
                    collapsed,
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

/// Navigable workspace entries in sidebar order.
pub(super) fn workspace_entries(
    snapshot: &ClientShellSnapshot,
    collapsed_groups: &HashSet<String>,
) -> Vec<WorkspaceEntry> {
    sidebar_rows(snapshot, collapsed_groups)
        .into_iter()
        .filter_map(|row| match row {
            SidebarRow::Workspace(entry) => Some(entry),
            SidebarRow::Marker(_) => None,
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
        key: marker.group.as_ref().map(|group| collapse_key(&group.name)),
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
            .fg(palette.subtext0)
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
