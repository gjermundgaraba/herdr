//! `client.report`: what this client's Agents list shows, sent to the Local
//! daemon so tools can read it through `client.list` and `client.changed`.
//!
//! The report is built only from the retained endpoint projections the
//! sidebar renders from, so its rows, their order, and their statuses are the
//! ones on screen. With one machine the sidebar draws the presented
//! projection, which trails the retained one only while a newer projection
//! waits for its pane frame, as during a reconnect. Nothing here draws: this
//! is mirrored client logic, and Herdr GPUI builds the same report from its
//! own retained snapshots (see "Sibling fork: Herdr GPUI" in `FORK.md`).

use std::hash::{Hash, Hasher};

use super::*;
use crate::api::schema::{
    ClientReport, ClientReportAgent, ClientReportEndpoint, ClientReportError,
    MAX_CLIENT_REPORT_AGENTS, MAX_CLIENT_REPORT_ENDPOINTS, MAX_CLIENT_REPORT_ID_BYTES,
};

impl From<ClientEndpointStatus> for crate::api::schema::ClientEndpointStatus {
    fn from(status: ClientEndpointStatus) -> Self {
        match status {
            ClientEndpointStatus::Connecting => Self::Connecting,
            ClientEndpointStatus::Online => Self::Online,
            ClientEndpointStatus::Reconnecting => Self::Reconnecting,
            ClientEndpointStatus::Attention => Self::Attention,
            ClientEndpointStatus::Disabled => Self::Disabled,
        }
    }
}

/// Whether the daemon accepts `id` in a report: it rejects the whole report
/// for one id beyond the limit.
fn within_id_limit(id: &str) -> bool {
    id.len() <= MAX_CLIENT_REPORT_ID_BYTES
}

impl ClientShellState {
    /// The report for the current projection, within the daemon's limits:
    /// endpoints and rows beyond the counts are left out, as is anything
    /// whose id the daemon would reject, and display text is cut.
    pub(crate) fn client_report(&self) -> Result<ClientReport, ClientReportError> {
        let listed = self
            .endpoints
            .iter()
            .filter(|endpoint| {
                endpoint
                    .snapshot
                    .as_deref()
                    .is_none_or(|snapshot| within_id_limit(&snapshot.boot_id))
            })
            .take(MAX_CLIENT_REPORT_ENDPOINTS)
            .collect::<Vec<_>>();
        let is_listed = |endpoint_id: &ClientEndpointId| {
            listed
                .iter()
                .any(|endpoint| &endpoint.endpoint_id == endpoint_id)
        };
        let mut focused_reported = false;
        let agents = aggregate_navigation::aggregate_agent_rows(
            &self.endpoints,
            &self.active_endpoint_id,
            self.config.agent_panel_sort,
        )
        .into_iter()
        .filter(|row| is_listed(row.endpoint.endpoint_id))
        .filter_map(|row| {
            let snapshot = row.endpoint.snapshot;
            let agent = row.agent;
            if agent.pane_id.is_empty()
                || ![&agent.pane_id, &agent.workspace_id, &agent.tab_id]
                    .into_iter()
                    .all(|id| within_id_limit(id))
            {
                return None;
            }
            // The sidebar draws no row for an agent whose space is unknown.
            let workspace = snapshot
                .workspaces
                .iter()
                .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
            let pane = snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == agent.pane_id);
            let focused = !focused_reported
                && row.endpoint.endpoint_id == &self.active_endpoint_id
                && agent.focused;
            focused_reported |= focused;
            Some(ClientReportAgent {
                endpoint_id: row.endpoint.endpoint_id.storage_key(),
                focused,
                workspace_label: workspace.label.clone(),
                workspace_number: workspace.number,
                cwd: pane.and_then(|pane| pane.cwd.clone()),
                foreground_cwd: pane.and_then(|pane| pane.foreground_cwd.clone()),
                agent: agent.clone(),
            })
        })
        .take(MAX_CLIENT_REPORT_AGENTS)
        .collect();
        let mut report = ClientReport {
            focused: self.outer_focused,
            active_endpoint_id: is_listed(&self.active_endpoint_id)
                .then(|| self.active_endpoint_id.storage_key()),
            endpoints: listed
                .iter()
                .map(|endpoint| ClientReportEndpoint {
                    endpoint_id: endpoint.endpoint_id.storage_key(),
                    label: endpoint.label.clone(),
                    status: endpoint.status.into(),
                    boot_id: endpoint
                        .snapshot
                        .as_deref()
                        .map(|snapshot| snapshot.boot_id.clone()),
                })
                .collect(),
            agents,
        };
        report.normalize()?;
        Ok(report)
    }

    /// Changes whenever [`Self::client_report`] may return something else.
    /// Scalars only: no row is built and nothing is sorted, so the client
    /// loop can afford it on every turn.
    pub(crate) fn client_report_fingerprint(&self) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.active_endpoint_id.hash(&mut hash);
        self.outer_focused.hash(&mut hash);
        std::mem::discriminant(&self.config.agent_panel_sort).hash(&mut hash);
        for endpoint in &self.endpoints {
            endpoint.endpoint_id.hash(&mut hash);
            endpoint.label.hash(&mut hash);
            std::mem::discriminant(&endpoint.status).hash(&mut hash);
            endpoint.snapshot_generation.hash(&mut hash);
            endpoint
                .snapshot
                .as_deref()
                .map(|snapshot| (&snapshot.boot_id, snapshot.revision))
                .hash(&mut hash);
            endpoint.agent_view_identity().hash(&mut hash);
        }
        hash.finish()
    }
}
