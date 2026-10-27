//! A client attached to several machines, for the tests of logic that Herdr
//! GPUI mirrors, such as Back/Forward. Nothing here draws. The
//! client loop's picker answer tests (`client_contract/tests.rs`) build their
//! shell state here too.

use super::*;
use crate::client::endpoint::{ClientEndpointStatus, ProfileId, SavedSshEndpoint};

/// An agent as a daemon reports it, its lifecycle sequence at `seq`.
pub(in crate::client) fn agent(pane_id: &str, status: AgentStatus, seq: u64) -> ClientShellAgent {
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
        state_change_seq: seq,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: false,
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

/// The enabled SSH profiles [`fleet`] configures, one per label.
pub(in crate::client) fn profiles(labels: &[&str]) -> Vec<SavedSshEndpoint> {
    labels
        .iter()
        .enumerate()
        .map(|(index, label)| SavedSshEndpoint {
            id: ProfileId::parse(format!("{:032x}", index + 1)).unwrap(),
            label: (*label).into(),
            target: format!("dev@{label}.example"),
            session: "agents".into(),
            enabled: true,
        })
        .collect()
}

/// A client on the `priority` sort with Local and one online SSH machine per
/// label, in that display order. No machine has sent a snapshot yet.
pub(in crate::client) fn fleet(labels: &[&str]) -> (ClientShellState, Vec<ClientEndpointId>) {
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
    let mut state = ClientShellState::new(config);
    let profiles = profiles(labels);
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
