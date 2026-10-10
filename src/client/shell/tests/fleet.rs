//! A client attached to several machines, for the tests of logic that Herdr
//! GPUI mirrors: the priority order, the client report, and jumps. Nothing
//! here draws. The client loop's contract tests (`client_contract/tests.rs`)
//! build their shell state here too.

use super::*;
use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};

/// An agent as a daemon reports it: `priority_key` already says where it
/// sorts. `stamp` is the daemon's ordering clock, and stands in for the
/// lifecycle sequence too.
pub(in crate::client) fn agent(pane_id: &str, status: AgentStatus, stamp: u64) -> ClientShellAgent {
    ClientShellAgent {
        state_change_seq: stamp,
        ..keyed_agent(
            pane_id,
            status,
            &crate::agent_priority::priority_key(false, status, stamp),
        )
    }
}

/// An agent in a space that carries a priority token.
pub(in crate::client) fn lifted_agent(
    pane_id: &str,
    status: AgentStatus,
    stamp: u64,
) -> ClientShellAgent {
    ClientShellAgent {
        state_change_seq: stamp,
        priority_mark: Some("★".into()),
        ..keyed_agent(
            pane_id,
            status,
            &crate::agent_priority::priority_key(true, status, stamp),
        )
    }
}

pub(in crate::client) fn keyed_agent(
    pane_id: &str,
    status: AgentStatus,
    key: &str,
) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some(pane_id.into()),
        display_agent: None,
        agent: Some("pi".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: status,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
        priority_key: key.into(),
        priority_mark: None,
    }
}

/// One machine's snapshot: a pane per agent, the first one focused.
pub(in crate::client) fn machine(
    boot_id: &str,
    agents: Vec<ClientShellAgent>,
) -> ClientShellSnapshot {
    let mut machine = snapshot();
    machine.boot_id = boot_id.into();
    let template = machine.panes[0].clone();
    machine.panes = agents
        .iter()
        .enumerate()
        .map(|(index, agent)| ClientShellPane {
            pane_id: agent.pane_id.clone(),
            focused: index == 0,
            ..template.clone()
        })
        .collect();
    machine.focused_pane_id = agents.first().map(|agent| agent.pane_id.clone());
    machine.agents = agents;
    if let Some(agent) = machine.agents.first_mut() {
        agent.focused = true;
    }
    machine
}

/// A client on the `priority` sort with Local and one online SSH machine per
/// label, in that display order. No machine has sent a snapshot yet.
pub(in crate::client) fn fleet(labels: &[&str]) -> (ClientShellState, Vec<ClientEndpointId>) {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    let mut state = ClientShellState::new(config);
    let profiles = labels
        .iter()
        .enumerate()
        .map(|(index, label)| SavedSshEndpoint {
            id: ProfileId::parse(format!("{:032x}", index + 1)).unwrap(),
            label: (*label).into(),
            target: format!("dev@{label}.example"),
            session: "agents".into(),
            enabled: true,
        })
        .collect::<Vec<_>>();
    state.set_endpoint_catalog(&profiles);
    let remotes = profiles
        .into_iter()
        .map(|profile| ClientEndpointId::Ssh(profile.id))
        .collect::<Vec<_>>();
    for remote in &remotes {
        state.set_endpoint_status(remote, ClientEndpointStatus::Online);
    }
    (state, remotes)
}

/// The snapshot the client retains for a machine.
pub(in crate::client) fn retained(
    state: &ClientShellState,
    endpoint_id: &ClientEndpointId,
) -> ClientShellSnapshot {
    state
        .endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .expect("retained snapshot")
        .clone()
}

/// The Agents list as `(machine label, pane id)`, in display order.
pub(in crate::client) fn listed(state: &ClientShellState) -> Vec<(String, String)> {
    aggregate_navigation::aggregate_agent_rows(
        &state.endpoints,
        &state.active_endpoint_id,
        state.config.agent_panel_sort,
    )
    .into_iter()
    .map(|row| (row.endpoint.label.to_owned(), row.agent.pane_id.clone()))
    .collect()
}
