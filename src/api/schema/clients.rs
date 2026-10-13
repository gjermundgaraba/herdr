//! Attached clients: the Agents list each one reports, and the jump a tool
//! asks one of them to perform.
//!
//! A client is one connection, keyed by the `client_id` the daemon assigned
//! it. A client that holds several connections reports on one of them and
//! answers jumps on that same connection.

use serde::{Deserialize, Serialize};

use crate::protocol::ClientShellAgent;

/// Endpoints one report may list; a longer list is rejected.
pub const MAX_CLIENT_REPORT_ENDPOINTS: usize = 64;
/// Agent rows one report may list; a longer list is rejected.
pub const MAX_CLIENT_REPORT_AGENTS: usize = 512;
/// Byte limit for the ids a report or jump carries; longer ids are rejected.
pub const MAX_CLIENT_REPORT_ID_BYTES: usize = 128;
/// Byte limit for display text in a report; longer text is truncated.
pub const MAX_CLIENT_REPORT_TEXT_BYTES: usize = 512;
/// State labels or tokens kept per agent row; the rest are dropped.
pub const MAX_CLIENT_REPORT_PAIRS: usize = 32;

/// What a client shows in its Agents list, across every endpoint it is
/// attached to. Each report replaces the previous one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientReport {
    /// Whether the client's OS window or outer terminal has focus; `null`
    /// until the client knows.
    pub focused: Option<bool>,
    /// The endpoint the client is presenting.
    pub active_endpoint_id: Option<String>,
    pub endpoints: Vec<ClientReportEndpoint>,
    /// The Agents list in the client's display order.
    pub agents: Vec<ClientReportAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientReportEndpoint {
    /// The client's own id for the endpoint, such as `local` or `ssh:<profile>`.
    pub endpoint_id: String,
    pub label: String,
    pub status: ClientEndpointStatus,
    /// Boot of the daemon behind the endpoint; `null` before its first snapshot.
    pub boot_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientEndpointStatus {
    Connecting,
    Online,
    Reconnecting,
    /// The endpoint needs the user, for example to sign in or update.
    Attention,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientReportAgent {
    pub endpoint_id: String,
    /// The agent the client has focused: the focused pane of the endpoint it
    /// presents. At most one row is focused; `agent.focused` is instead the
    /// focus inside the row's own endpoint.
    pub focused: bool,
    /// Label and number of the space the agent belongs to.
    pub workspace_label: String,
    pub workspace_number: usize,
    /// The pane's working directory, when its endpoint knows it.
    pub cwd: Option<String>,
    pub foreground_cwd: Option<String>,
    /// The row's record from its endpoint's snapshot, as the client shows it.
    pub agent: ClientShellAgent,
}

/// Why a report was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientReportError {
    TooManyEndpoints,
    TooManyAgents,
    IdTooLong,
    EmptyId,
    DuplicateEndpoint,
    UnknownActiveEndpoint,
    UnknownAgentEndpoint,
    MultipleFocusedAgents,
}

impl ClientReportError {
    pub fn message(self) -> &'static str {
        match self {
            Self::TooManyEndpoints => "a report lists at most 64 endpoints",
            Self::TooManyAgents => "a report lists at most 512 agents",
            Self::IdTooLong => "endpoint, boot, workspace, tab, and pane ids are at most 128 bytes",
            Self::EmptyId => "endpoint and pane ids must not be empty",
            Self::DuplicateEndpoint => "an endpoint id is listed more than once",
            Self::UnknownActiveEndpoint => "active_endpoint_id names an endpoint the report omits",
            Self::UnknownAgentEndpoint => "an agent names an endpoint the report omits",
            Self::MultipleFocusedAgents => "at most one agent is focused",
        }
    }
}

impl ClientReport {
    /// Checks the report's structure and bounds its display text.
    pub fn normalize(&mut self) -> Result<(), ClientReportError> {
        if self.endpoints.len() > MAX_CLIENT_REPORT_ENDPOINTS {
            return Err(ClientReportError::TooManyEndpoints);
        }
        if self.agents.len() > MAX_CLIENT_REPORT_AGENTS {
            return Err(ClientReportError::TooManyAgents);
        }
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            check_id(&endpoint.endpoint_id)?;
            if let Some(boot_id) = &endpoint.boot_id {
                check_id_length(boot_id)?;
            }
            if self.endpoints[..index]
                .iter()
                .any(|earlier| earlier.endpoint_id == endpoint.endpoint_id)
            {
                return Err(ClientReportError::DuplicateEndpoint);
            }
        }
        let listed = |endpoint_id: &str| {
            self.endpoints
                .iter()
                .any(|endpoint| endpoint.endpoint_id == endpoint_id)
        };
        if self
            .active_endpoint_id
            .as_deref()
            .is_some_and(|endpoint_id| !listed(endpoint_id))
        {
            return Err(ClientReportError::UnknownActiveEndpoint);
        }
        if self.agents.iter().any(|row| !listed(&row.endpoint_id)) {
            return Err(ClientReportError::UnknownAgentEndpoint);
        }
        if self.agents.iter().filter(|row| row.focused).count() > 1 {
            return Err(ClientReportError::MultipleFocusedAgents);
        }
        for row in &self.agents {
            check_id(&row.agent.pane_id)?;
            check_id_length(&row.agent.workspace_id)?;
            check_id_length(&row.agent.tab_id)?;
        }
        for endpoint in &mut self.endpoints {
            truncate(&mut endpoint.label);
        }
        for row in &mut self.agents {
            truncate(&mut row.workspace_label);
            for text in [&mut row.cwd, &mut row.foreground_cwd] {
                truncate_optional(text);
            }
            let agent = &mut row.agent;
            for text in [
                &mut agent.name,
                &mut agent.display_agent,
                &mut agent.agent,
                &mut agent.title,
                &mut agent.terminal_title,
                &mut agent.terminal_title_stripped,
                &mut agent.priority_mark,
            ] {
                truncate_optional(text);
            }
            truncate(&mut agent.priority_key);
            for pairs in [&mut agent.state_labels, &mut agent.tokens] {
                pairs.truncate(MAX_CLIENT_REPORT_PAIRS);
                for (key, value) in pairs {
                    truncate(key);
                    truncate(value);
                }
            }
        }
        Ok(())
    }
}

fn check_id(id: &str) -> Result<(), ClientReportError> {
    if id.is_empty() {
        return Err(ClientReportError::EmptyId);
    }
    check_id_length(id)
}

fn check_id_length(id: &str) -> Result<(), ClientReportError> {
    if id.len() > MAX_CLIENT_REPORT_ID_BYTES {
        return Err(ClientReportError::IdTooLong);
    }
    Ok(())
}

fn truncate_optional(text: &mut Option<String>) {
    if let Some(text) = text {
        truncate(text);
    }
}

/// Cuts `text` to the report's text limit on a character boundary.
fn truncate(text: &mut String) {
    text.truncate(text.floor_char_boundary(MAX_CLIENT_REPORT_TEXT_BYTES));
}

/// One attached client, as `client.list` describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientInfo {
    pub client_id: u64,
    /// The client's latest report; absent until it sends one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ClientReport>,
}

/// Asks one client to show a pane, switching endpoint when it has to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientJumpParams {
    pub client_id: u64,
    /// The endpoint, in the ids of that client's report.
    pub endpoint_id: String,
    /// The endpoint's boot the pane id was read from.
    pub boot_id: String,
    pub pane_id: String,
}

/// How a client's jump ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientJumpOutcome {
    /// The client presents the endpoint with the pane focused.
    Focused,
    /// The endpoint now runs another boot, so the pane id may name another pane.
    StaleBoot,
    /// The client knows no such endpoint, or the endpoint no such pane.
    NotFound,
    /// The endpoint is not online, or has sent the client no snapshot yet.
    Unavailable,
    /// The client did not see the pane focused in time; it may still happen.
    Timeout,
}

/// Sent by the client that received the jump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ClientJumpResolveParams {
    pub jump_id: String,
    pub outcome: ClientJumpOutcome,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::AgentStatus;

    fn agent(pane_id: &str) -> ClientShellAgent {
        ClientShellAgent {
            pane_id: pane_id.into(),
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            name: None,
            display_agent: None,
            agent: Some("claude".into()),
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: AgentStatus::Idle,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: false,
            priority_key: String::new(),
            priority_mark: None,
        }
    }

    fn row(endpoint_id: &str, pane_id: &str) -> ClientReportAgent {
        ClientReportAgent {
            endpoint_id: endpoint_id.into(),
            focused: false,
            workspace_label: "api".into(),
            workspace_number: 1,
            cwd: None,
            foreground_cwd: None,
            agent: agent(pane_id),
        }
    }

    fn report() -> ClientReport {
        ClientReport {
            focused: Some(true),
            active_endpoint_id: Some("local".into()),
            endpoints: vec![ClientReportEndpoint {
                endpoint_id: "local".into(),
                label: "Local".into(),
                status: ClientEndpointStatus::Online,
                boot_id: Some("boot".into()),
            }],
            agents: vec![row("local", "w1:p1")],
        }
    }

    #[test]
    fn a_well_formed_report_is_kept_as_sent() {
        let mut normalized = report();
        assert_eq!(normalized.normalize(), Ok(()));
        assert_eq!(normalized, report());
    }

    #[test]
    fn structural_mistakes_are_rejected() {
        let mut too_many_endpoints = report();
        too_many_endpoints.endpoints = (0..=MAX_CLIENT_REPORT_ENDPOINTS)
            .map(|index| ClientReportEndpoint {
                endpoint_id: format!("ssh:{index}"),
                label: String::new(),
                status: ClientEndpointStatus::Online,
                boot_id: None,
            })
            .collect();
        assert_eq!(
            too_many_endpoints.normalize(),
            Err(ClientReportError::TooManyEndpoints)
        );

        let mut too_many_agents = report();
        too_many_agents.agents = (0..=MAX_CLIENT_REPORT_AGENTS)
            .map(|index| row("local", &format!("w1:p{index}")))
            .collect();
        assert_eq!(
            too_many_agents.normalize(),
            Err(ClientReportError::TooManyAgents)
        );

        let mut duplicate = report();
        duplicate.endpoints.push(duplicate.endpoints[0].clone());
        assert_eq!(
            duplicate.normalize(),
            Err(ClientReportError::DuplicateEndpoint)
        );

        let mut unknown_active = report();
        unknown_active.active_endpoint_id = Some("ssh:gone".into());
        assert_eq!(
            unknown_active.normalize(),
            Err(ClientReportError::UnknownActiveEndpoint)
        );

        let mut unknown_agent_endpoint = report();
        unknown_agent_endpoint.agents.push(row("ssh:gone", "w1:p2"));
        assert_eq!(
            unknown_agent_endpoint.normalize(),
            Err(ClientReportError::UnknownAgentEndpoint)
        );

        let mut two_focused = report();
        two_focused.agents.push(row("local", "w1:p2"));
        for row in &mut two_focused.agents {
            row.focused = true;
        }
        assert_eq!(
            two_focused.normalize(),
            Err(ClientReportError::MultipleFocusedAgents)
        );

        let mut long_id = report();
        long_id.agents[0].agent.pane_id = "p".repeat(MAX_CLIENT_REPORT_ID_BYTES + 1);
        assert_eq!(long_id.normalize(), Err(ClientReportError::IdTooLong));

        let mut empty_id = report();
        empty_id.agents[0].agent.pane_id.clear();
        assert_eq!(empty_id.normalize(), Err(ClientReportError::EmptyId));
    }

    #[test]
    fn display_text_is_cut_on_a_character_boundary() {
        let mut long = report();
        // One byte, then two-byte characters: the byte limit falls inside one.
        long.agents[0].agent.title = Some(format!("a{}", "é".repeat(MAX_CLIENT_REPORT_TEXT_BYTES)));
        long.agents[0].agent.tokens = (0..MAX_CLIENT_REPORT_PAIRS + 5)
            .map(|index| {
                (
                    format!("k{index}"),
                    "v".repeat(MAX_CLIENT_REPORT_TEXT_BYTES * 2),
                )
            })
            .collect();
        long.endpoints[0].label = "x".repeat(MAX_CLIENT_REPORT_TEXT_BYTES + 1);

        assert_eq!(long.normalize(), Ok(()));

        let title = long.agents[0].agent.title.as_deref().unwrap();
        assert_eq!(title.len(), MAX_CLIENT_REPORT_TEXT_BYTES - 1);
        assert!(title.starts_with('a') && title.chars().skip(1).all(|ch| ch == 'é'));
        assert_eq!(long.agents[0].agent.tokens.len(), MAX_CLIENT_REPORT_PAIRS);
        assert_eq!(
            long.agents[0].agent.tokens[0].1.len(),
            MAX_CLIENT_REPORT_TEXT_BYTES
        );
        assert_eq!(long.endpoints[0].label.len(), MAX_CLIENT_REPORT_TEXT_BYTES);
    }
}
