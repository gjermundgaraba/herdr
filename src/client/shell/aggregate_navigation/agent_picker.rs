//! The agent picker: every agent across endpoints as one flat list in the
//! `priority` order, each row marked with its space's priority mark.
//!
//! Mirrored client logic: Herdr GPUI's agent picker is
//! `crates/herdr-gpui/src/picker/navigator/rows.rs` (see "Sibling fork:
//! Herdr GPUI" in `FORK.md`).

use super::*;

/// Every agent pane across machines as one flat list in priority order.
pub(super) fn agent_navigator_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    filter: &impl Fn(crate::api::schema::AgentStatus) -> bool,
    text: &impl Fn(&str) -> bool,
) -> Vec<ClientNavigatorRow> {
    let federated = endpoints.len() > 1;
    aggregate_agent_rows(
        endpoints,
        active_endpoint_id,
        crate::config::AgentPanelSortConfig::Priority,
    )
    .into_iter()
    .filter_map(|row| {
        let agent = row.agent;
        let snapshot = row.endpoint.snapshot;
        if !filter(agent.agent_status) {
            return None;
        }
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == agent.pane_id);
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == agent.workspace_id);
        let mark = agent.priority_mark.clone();
        let workspace = workspace.map_or(agent.workspace_id.as_str(), |workspace| {
            workspace.label.as_str()
        });
        let name = pane
            .and_then(|pane| pane.label.clone())
            .or_else(|| agent.name.clone())
            .or_else(|| agent.display_agent.clone())
            .or_else(|| agent.title.clone())
            .unwrap_or_else(|| agent.pane_id.clone());
        let label = if federated && !row.endpoint.endpoint_id.is_local() {
            format!("{} · {workspace}: {name}", row.endpoint.label)
        } else {
            format!("{workspace}: {name}")
        };
        let meta = pane
            .and_then(|pane| pane.foreground_cwd.clone().or_else(|| pane.cwd.clone()))
            .unwrap_or_default();
        if !(text(&label) || text(&meta)) {
            return None;
        }
        let tab = pane
            .and_then(|pane| snapshot.tabs.iter().find(|tab| tab.tab_id == pane.tab_id))
            .map_or("", |tab| tab.label.as_str());
        Some(ClientNavigatorRow {
            depth: 0,
            label,
            meta,
            detail: format!("{workspace} / {tab} / {}", agent.pane_id),
            agent: agent.agent.clone().or_else(|| agent.display_agent.clone()),
            status: Some(agent.agent_status),
            stale: row.endpoint.stale(),
            current: row.endpoint.endpoint_id == active_endpoint_id
                && snapshot.focused_pane_id.as_deref() == Some(agent.pane_id.as_str()),
            mark,
            target: ClientNavigatorTarget::Pane {
                endpoint_id: row.endpoint.endpoint_id.clone(),
                pane_id: agent.pane_id.clone(),
            },
        })
    })
    .collect()
}
