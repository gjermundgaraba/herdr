//! `client.report` as the shell builds it: the Agents list this client
//! shows, read from its retained projection (`client_report.rs`).

use super::fleet::{agent, fleet, lifted_agent, listed, machine};
use super::*;
use crate::api::schema::{
    ClientEndpointStatus as ReportedStatus, ClientReport, ClientReportEndpoint,
    MAX_CLIENT_REPORT_AGENTS, MAX_CLIENT_REPORT_ENDPOINTS, MAX_CLIENT_REPORT_ID_BYTES,
    MAX_CLIENT_REPORT_TEXT_BYTES,
};
use crate::client::endpoint::ClientEndpointStatus;
use crate::config::AgentPanelSortConfig;

/// Local with a waiting and an idle agent, Build with a lifted blocked one.
fn two_machines() -> (ClientShellState, ClientEndpointId) {
    let (mut state, remotes) = fleet(&["Build"]);
    let mut local = machine(
        "boot-1",
        vec![
            agent("local-idle", AgentStatus::Idle, 5),
            agent("local-blocked", AgentStatus::Blocked, 20),
        ],
    );
    local.panes[1].cwd = Some("/repo/api".into());
    local.panes[1].foreground_cwd = Some("/repo/api/src".into());
    state.set_snapshot(Box::new(local));
    let mut build = machine(
        "build-boot",
        vec![lifted_agent("build-lifted", AgentStatus::Blocked, 40)],
    );
    build.workspaces[0].label = "release".into();
    build.workspaces[0].number = 3;
    state.set_endpoint_snapshot(&remotes[0], Box::new(build));
    (state, remotes[0].clone())
}

fn report(state: &ClientShellState) -> ClientReport {
    state.client_report().expect("report within limits")
}

fn reported(report: &ClientReport) -> Vec<(String, String)> {
    report
        .agents
        .iter()
        .map(|row| (row.endpoint_id.clone(), row.agent.pane_id.clone()))
        .collect()
}

#[test]
fn the_report_lists_every_machine_and_the_agents_as_displayed() {
    let (state, remote) = two_machines();
    let remote_key = remote.storage_key();

    let report = report(&state);

    assert_eq!(report.focused, None, "no focus event has arrived yet");
    assert_eq!(report.active_endpoint_id.as_deref(), Some("local"));
    assert_eq!(
        report.endpoints,
        [
            ClientReportEndpoint {
                endpoint_id: "local".into(),
                label: "Local".into(),
                status: ReportedStatus::Online,
                boot_id: Some("boot-1".into()),
            },
            ClientReportEndpoint {
                endpoint_id: remote_key.clone(),
                label: "Build".into(),
                status: ReportedStatus::Online,
                boot_id: Some("build-boot".into()),
            },
        ]
    );
    assert_eq!(
        reported(&report),
        [
            (remote_key.clone(), "build-lifted".to_owned()),
            ("local".to_owned(), "local-blocked".to_owned()),
            ("local".to_owned(), "local-idle".to_owned()),
        ]
    );

    let lifted = &report.agents[0];
    assert_eq!(lifted.workspace_label, "release");
    assert_eq!(lifted.workspace_number, 3);
    assert_eq!(lifted.agent.agent_status, AgentStatus::Blocked);
    assert_eq!(lifted.agent.priority_mark.as_deref(), Some("★"));
    assert_eq!(
        lifted.agent.priority_key,
        crate::agent_priority::priority_key(true, AgentStatus::Blocked, 40)
    );
    let blocked = &report.agents[1];
    assert_eq!(blocked.cwd.as_deref(), Some("/repo/api"));
    assert_eq!(blocked.foreground_cwd.as_deref(), Some("/repo/api/src"));
    assert_eq!(blocked.workspace_label, "client-shell");
}

#[test]
fn rows_follow_the_displayed_order_under_either_sort() {
    let (mut state, _) = two_machines();
    let ids = |state: &ClientShellState| {
        let labels = listed(state);
        let rows = reported(&report(state));
        assert_eq!(labels.len(), rows.len());
        rows.into_iter().map(|(_, pane)| pane).collect::<Vec<_>>()
    };

    assert_eq!(ids(&state), ["build-lifted", "local-blocked", "local-idle"]);
    assert_eq!(
        ids(&state),
        listed(&state)
            .into_iter()
            .map(|(_, pane)| pane)
            .collect::<Vec<_>>()
    );

    state.config.agent_panel_sort = AgentPanelSortConfig::Spaces;
    assert_eq!(ids(&state), ["local-idle", "local-blocked", "build-lifted"]);
}

#[test]
fn only_the_presented_machines_focused_pane_is_the_focused_row() {
    let (mut state, remote) = two_machines();
    let focused = |state: &ClientShellState| {
        report(state)
            .agents
            .iter()
            .filter(|row| row.focused)
            .map(|row| (row.endpoint_id.clone(), row.agent.pane_id.clone()))
            .collect::<Vec<_>>()
    };

    // Build also has a focused pane of its own, but it is not presented.
    assert_eq!(
        focused(&state),
        [("local".to_owned(), "local-idle".to_owned())]
    );

    assert!(state.activate_endpoint_projection(&remote));
    assert_eq!(
        focused(&state),
        [(remote.storage_key(), "build-lifted".to_owned())]
    );
    assert_eq!(
        report(&state).active_endpoint_id,
        Some(remote.storage_key())
    );
}

#[test]
fn window_focus_is_reported_once_the_terminal_says() {
    let (mut state, _) = two_machines();
    assert_eq!(report(&state).focused, None);

    state.outer_focused = Some(false);
    assert_eq!(report(&state).focused, Some(false));
    // The focused row says where the client is, focused window or not.
    assert_eq!(
        report(&state)
            .agents
            .iter()
            .filter(|row| row.focused)
            .count(),
        1
    );

    state.outer_focused = Some(true);
    assert_eq!(report(&state).focused, Some(true));
}

#[test]
fn rows_carry_each_daemons_agent_record_unchanged() {
    let (mut state, remotes) = fleet(&["Build"]);
    state.set_snapshot(Box::new(machine("boot-1", Vec::new())));
    let mut build = machine(
        "build-boot",
        vec![agent("build-agent", AgentStatus::Working, 1)],
    );
    state.set_endpoint_snapshot(&remotes[0], Box::new(build.clone()));
    // The agent finishes. Whether that is a badge is the daemon's call: it
    // reports `done` until one of its clients has seen the completion, then
    // `idle`, and this client shows and reports either as sent, key included.
    for (revision, status) in [(2, AgentStatus::Done), (3, AgentStatus::Idle)] {
        build.revision = revision;
        build.agents = vec![agent("build-agent", status, 2)];
        let sent = build.agents[0].clone();
        state.set_endpoint_snapshot(&remotes[0], Box::new(build.clone()));

        assert_eq!(report(&state).agents[0].agent, sent);
    }
}

#[test]
fn a_machine_without_a_snapshot_is_listed_without_boot_or_rows() {
    let (mut state, remotes) = fleet(&["Build", "Lab"]);
    state.set_snapshot(Box::new(machine(
        "boot-1",
        vec![agent("local-idle", AgentStatus::Idle, 1)],
    )));
    state.set_endpoint_snapshot(
        &remotes[1],
        Box::new(machine(
            "lab-boot",
            vec![agent("lab-done", AgentStatus::Done, 2)],
        )),
    );
    state.set_endpoint_status(&remotes[0], ClientEndpointStatus::Connecting);
    state.set_endpoint_status(&remotes[1], ClientEndpointStatus::Reconnecting);

    let report = report(&state);

    assert_eq!(
        report
            .endpoints
            .iter()
            .map(|endpoint| (
                endpoint.label.as_str(),
                endpoint.status,
                endpoint.boot_id.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("Local", ReportedStatus::Online, Some("boot-1")),
            ("Build", ReportedStatus::Connecting, None),
            ("Lab", ReportedStatus::Reconnecting, Some("lab-boot")),
        ]
    );
    // The reconnecting machine's retained rows are still shown, after the
    // online ones.
    assert_eq!(
        reported(&report),
        [
            ("local".to_owned(), "local-idle".to_owned()),
            (remotes[1].storage_key(), "lab-done".to_owned()),
        ]
    );
}

#[test]
fn the_report_stays_within_the_daemons_limits() {
    let labels = (0..MAX_CLIENT_REPORT_ENDPOINTS + 3)
        .map(|index| format!("machine-{index}"))
        .collect::<Vec<_>>();
    let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let (mut state, remotes) = fleet(&labels);
    let mut local = machine(
        "boot-1",
        (0..MAX_CLIENT_REPORT_AGENTS + 10)
            .map(|index| agent(&format!("pane_{index:04}"), AgentStatus::Done, index as u64))
            .collect(),
    );
    local.workspaces[0].label = "é".repeat(MAX_CLIENT_REPORT_TEXT_BYTES);
    // An id the daemon would reject the whole report for.
    local.agents[1].pane_id = "p".repeat(MAX_CLIENT_REPORT_ID_BYTES + 1);
    state.set_snapshot(Box::new(local));
    // The last machine is beyond the endpoint limit, so its rows go too.
    let beyond = remotes.last().expect("remote");
    state.set_endpoint_snapshot(
        beyond,
        Box::new(machine(
            "far-boot",
            vec![lifted_agent("far-lifted", AgentStatus::Blocked, 1)],
        )),
    );

    let report = report(&state);

    assert_eq!(report.endpoints.len(), MAX_CLIENT_REPORT_ENDPOINTS);
    assert!(report
        .endpoints
        .iter()
        .all(|endpoint| endpoint.endpoint_id != beyond.storage_key()));
    assert_eq!(report.agents.len(), MAX_CLIENT_REPORT_AGENTS);
    assert!(report.agents.iter().all(|row| row.endpoint_id == "local"));
    // The first rows of the list are the ones kept, less the rejected id.
    assert_eq!(report.agents[0].agent.pane_id, "pane_0000");
    assert_eq!(report.agents[1].agent.pane_id, "pane_0002");
    assert_eq!(
        report.agents[0].workspace_label.len(),
        MAX_CLIENT_REPORT_TEXT_BYTES
    );
    let mut again = report.clone();
    assert_eq!(again.normalize(), Ok(()));
    assert_eq!(again, report);
}

#[test]
fn presenting_a_machine_beyond_the_limit_reports_no_active_endpoint() {
    let labels = (0..MAX_CLIENT_REPORT_ENDPOINTS)
        .map(|index| format!("machine-{index}"))
        .collect::<Vec<_>>();
    let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
    let (mut state, remotes) = fleet(&labels);
    state.set_snapshot(Box::new(machine("boot-1", Vec::new())));
    let beyond = remotes.last().expect("remote");
    state.set_endpoint_snapshot(
        beyond,
        Box::new(machine(
            "far-boot",
            vec![agent("far", AgentStatus::Idle, 1)],
        )),
    );
    assert!(state.activate_endpoint_projection(beyond));

    let report = report(&state);

    assert_eq!(report.active_endpoint_id, None);
    assert!(report.agents.is_empty());
}

#[test]
fn the_fingerprint_follows_everything_the_report_reads() {
    let (mut state, remote) = two_machines();
    let mut seen = vec![state.client_report_fingerprint()];
    let mut changed = |state: &ClientShellState, what: &str| {
        let fingerprint = state.client_report_fingerprint();
        assert!(!seen.contains(&fingerprint), "{what} left it unchanged");
        seen.push(fingerprint);
    };

    state.outer_focused = Some(true);
    changed(&state, "window focus");

    state.config.agent_panel_sort = AgentPanelSortConfig::Spaces;
    changed(&state, "the sort");

    state.set_endpoint_status(&remote, ClientEndpointStatus::Reconnecting);
    changed(&state, "a machine's status");

    let mut next = state.snapshot.as_deref().expect("snapshot").clone();
    next.revision += 1;
    next.agents[0].title = Some("renamed".into());
    state.set_snapshot(Box::new(next));
    changed(&state, "a snapshot");

    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    assert!(state.activate_endpoint_projection(&remote));
    changed(&state, "the presented machine");
}

#[test]
fn the_fingerprint_is_stable_while_nothing_reported_changes() {
    let (mut state, _) = two_machines();
    state.set_pane_surface(surface());
    let before = state.client_report_fingerprint();
    let report_before = report(&state);

    // Drawing, scrolling, and opening an overlay report nothing new.
    state.compose(120, 40).expect("frame");
    state.agent_scroll = 1;
    state.open_navigator(NavigatorLayout::Agents);
    state.compose(120, 40).expect("frame");

    assert_eq!(state.client_report_fingerprint(), before);
    assert_eq!(report(&state), report_before);
}
