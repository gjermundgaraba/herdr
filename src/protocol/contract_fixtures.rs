//! The fixtures for the wire shapes this fork adds between the daemon, its
//! clients, and the tools that read client reports. Herdr GPUI's protocol
//! crate keeps verbatim copies of these files, so each test pins a fixture to
//! the exact value the daemon decodes and to its canonical encoding.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::endpoint::{client_jump_message, EndpointClientJump, CLIENT_JUMP_KIND};
use super::{
    ClientShellAgent, ClientShellCommand, ClientShellCommandAction, ClientShellPane,
    ClientShellPinnedSection, ClientShellPluginEntry, ClientShellSnapshot, ClientShellTab,
    ClientShellWorkspace, ServerMessage,
};
use crate::agent_priority::priority_key;
use crate::api::schema::{
    AgentStatus, ClientEndpointStatus, ClientInfo, ClientJumpOutcome, ClientJumpParams,
    ClientJumpResolveParams, ClientReport, ClientReportAgent, ClientReportEndpoint, EventData,
    EventEnvelope, EventKind, Method, PaneTarget, PluginActionContext, Request, ResponseResult,
    SuccessResponse,
};

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/",
            $name
        ))
    };
}

/// The fixture decodes to `expected`, and `expected` encodes to exactly the
/// fixture's JSON: no field more, none missing.
fn assert_canonical<T>(fixture: &str, expected: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json: serde_json::Value = serde_json::from_str(fixture).expect("fixture is JSON");
    let decoded: T = serde_json::from_value(json.clone()).expect("fixture decodes");
    assert_eq!(&decoded, expected);
    assert_eq!(serde_json::to_value(expected).expect("value encodes"), json);
}

// Stamps of the priority clock: microseconds since the Unix epoch.
const BLOCKED_AT: u64 = 1_791_590_400_000_000;
const DONE_AT: u64 = 1_791_590_460_500_000;
const IDLE_AT: u64 = 1_791_590_520_000_000;

fn agent(
    pane_id: &str,
    workspace_id: &str,
    status: AgentStatus,
    state_change_seq: u64,
    stamp: u64,
    mark: Option<&str>,
) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: workspace_id.into(),
        tab_id: format!("{workspace_id}:t1"),
        name: None,
        display_agent: Some("Claude".into()),
        agent: Some("claude".into()),
        title: Some("Review".into()),
        terminal_title: Some("Claude Review".into()),
        terminal_title_stripped: Some("Claude Review".into()),
        agent_status: status,
        state_change_seq,
        state_labels: Vec::new(),
        tokens: vec![("task".into(), "review".into())],
        focused: pane_id == "w2:p1",
        priority_key: priority_key(mark.is_some(), status, stamp),
        priority_mark: mark.map(str::to_owned),
    }
}

fn agents() -> Vec<ClientShellAgent> {
    vec![
        agent("w1:p1", "w1", AgentStatus::Done, 12, DONE_AT, Some("high")),
        agent("w2:p1", "w2", AgentStatus::Blocked, 11, BLOCKED_AT, None),
        agent("w2:p2", "w2", AgentStatus::Idle, 13, IDLE_AT, None),
    ]
}

fn workspace(workspace_id: &str, number: usize, label: &str, lifted: bool) -> ClientShellWorkspace {
    ClientShellWorkspace {
        workspace_id: workspace_id.into(),
        active_tab_id: format!("{workspace_id}:t1"),
        new_workspace_cwd: format!("/work/{label}"),
        number,
        label: label.into(),
        custom_label: false,
        branch: Some("main".into()),
        git_ahead_behind: None,
        tokens: if lifted {
            vec![
                ("prio".into(), "high".into()),
                ("team".into(), "platform".into()),
            ]
        } else {
            vec![("team".into(), "platform".into())]
        },
        worktree: None,
        focused: !lifted,
        agent_status: if lifted {
            AgentStatus::Done
        } else {
            AgentStatus::Blocked
        },
    }
}

fn snapshot() -> ClientShellSnapshot {
    ClientShellSnapshot {
        boot_id: "boot-a".into(),
        revision: 7,
        config_diagnostic: None,
        product_announcement: None,
        update_available: None,
        update_install_command: "herdr update".into(),
        server_keybindings_toml: None,
        latest_release_notes_available: false,
        integration_updates_available: false,
        worktree_directory: "/worktrees".into(),
        release_notes: None,
        focused_workspace_id: Some("w2".into()),
        focused_tab_id: Some("w2:t1".into()),
        focused_pane_id: Some("w2:p1".into()),
        tab_bar_right: Vec::new(),
        tab_bar_right_separator: " ".into(),
        agent_view_label: None,
        agent_order: vec!["w1:p1".into(), "w2:p1".into(), "w2:p2".into()],
        workspaces: vec![
            workspace("w1", 1, "api", true),
            workspace("w2", 2, "web", false),
        ],
        tabs: ["w1", "w2"]
            .into_iter()
            .map(|workspace_id| ClientShellTab {
                tab_id: format!("{workspace_id}:t1"),
                workspace_id: workspace_id.into(),
                number: 1,
                label: "main".into(),
                custom_label: false,
                zoomed: false,
                focused: workspace_id == "w2",
                agent_status: if workspace_id == "w1" {
                    AgentStatus::Done
                } else {
                    AgentStatus::Blocked
                },
            })
            .collect(),
        panes: agents()
            .into_iter()
            .map(|agent| ClientShellPane {
                focused: agent.focused,
                cwd: Some(format!("/work/{}", agent.workspace_id)),
                foreground_cwd: None,
                label: None,
                right_click_passthrough: false,
                pane_id: agent.pane_id,
                workspace_id: agent.workspace_id,
                tab_id: agent.tab_id,
            })
            .collect(),
        agents: agents(),
        commands: vec![ClientShellCommand {
            command_id: "deploy".into(),
            binding_label: "prefix+d".into(),
            binding_labels: vec!["prefix+d".into()],
            action: ClientShellCommandAction::Shell,
            description: None,
        }],
        plugin_entries: vec![ClientShellPluginEntry {
            plugin_id: "example.plugin".into(),
            entry_id: "toggle".into(),
            title: "Toggle".into(),
            contexts: vec![PluginActionContext::Workspace],
            popup: false,
        }],
        pinned_sections: vec![
            ClientShellPinnedSection {
                title: "Priority".into(),
                fg: Some("#ff8800".into()),
                workspace_ids: vec!["w1".into()],
            },
            ClientShellPinnedSection {
                title: "Platform".into(),
                fg: None,
                workspace_ids: vec!["w1".into(), "w2".into()],
            },
        ],
    }
}

fn report() -> ClientReport {
    let local = agents();
    let mut remote = agent("w1:p4", "w1", AgentStatus::Working, 3, BLOCKED_AT, None);
    remote.focused = true;
    let row = |endpoint_id: &str, label: &str, number: usize, agent: ClientShellAgent| {
        ClientReportAgent {
            endpoint_id: endpoint_id.into(),
            focused: endpoint_id == "local" && agent.focused,
            workspace_label: label.into(),
            workspace_number: number,
            cwd: Some(format!("/work/{label}")),
            foreground_cwd: None,
            agent,
        }
    };
    ClientReport {
        focused: Some(true),
        active_endpoint_id: Some("local".into()),
        endpoints: vec![
            ClientReportEndpoint {
                endpoint_id: "local".into(),
                label: "Local".into(),
                status: ClientEndpointStatus::Online,
                boot_id: Some("boot-a".into()),
            },
            ClientReportEndpoint {
                endpoint_id: "ssh:0123456789abcdef0123456789abcdef".into(),
                label: "Build".into(),
                status: ClientEndpointStatus::Online,
                boot_id: Some("boot-b".into()),
            },
            ClientReportEndpoint {
                endpoint_id: "ssh:fedcba9876543210fedcba9876543210".into(),
                label: "Staging".into(),
                status: ClientEndpointStatus::Reconnecting,
                boot_id: None,
            },
        ],
        // Display order: the keys of both endpoints, merged.
        agents: vec![
            row("local", "api", 1, local[0].clone()),
            row("local", "web", 2, local[1].clone()),
            row("ssh:0123456789abcdef0123456789abcdef", "infra", 1, remote),
            row("local", "web", 2, local[2].clone()),
        ],
    }
}

#[test]
fn snapshot_fixture_carries_pinned_sections_and_agent_priority() {
    let expected = snapshot();
    assert_canonical(fixture!("endpoint-snapshot-shared-v1.json"), &expected);

    // Sorting by key gives the daemon's own priority order, and the lifted
    // done agent leads although a blocked agent is waiting elsewhere.
    let mut by_key = expected.agents.iter().collect::<Vec<_>>();
    by_key.sort_by(|left, right| left.priority_key.cmp(&right.priority_key));
    assert_eq!(
        by_key
            .iter()
            .map(|agent| agent.pane_id.as_str())
            .collect::<Vec<_>>(),
        expected.agent_order
    );
    assert_eq!(expected.agents[0].priority_key, "0.1.00065d7128fb2820");
}

#[test]
fn a_snapshot_without_the_additions_still_decodes() {
    let mut json: serde_json::Value =
        serde_json::from_str(fixture!("endpoint-snapshot-shared-v1.json")).unwrap();
    json.as_object_mut().unwrap().remove("pinned_sections");
    for agent in json["agents"].as_array_mut().unwrap() {
        agent.as_object_mut().unwrap().remove("priority_key");
        agent.as_object_mut().unwrap().remove("priority_mark");
    }
    let decoded: ClientShellSnapshot = serde_json::from_value(json).unwrap();
    assert!(decoded.pinned_sections.is_empty());
    assert!(decoded
        .agents
        .iter()
        .all(|agent| agent.priority_key.is_empty() && agent.priority_mark.is_none()));
}

#[test]
fn client_report_fixture_is_the_lane_request() {
    let expected = Request {
        id: "client-shell:7".into(),
        method: Method::ClientReport(report()),
    };
    assert_canonical(fixture!("endpoint-client-report-v1.json"), &expected);

    let Method::ClientReport(mut normalized) = expected.method.clone() else {
        unreachable!();
    };
    assert_eq!(normalized.normalize(), Ok(()));
    assert_eq!(Method::ClientReport(normalized), expected.method);
}

#[test]
fn client_jump_fixture_is_the_control_payload() {
    let expected = EndpointClientJump {
        jump_id: "jump-3".into(),
        endpoint_id: "ssh:0123456789abcdef0123456789abcdef".into(),
        boot_id: "boot-b".into(),
        pane_id: "w1:p4".into(),
    };
    assert_canonical(fixture!("endpoint-client-jump-v1.json"), &expected);

    let ServerMessage::EndpointControl { kind, data } = client_jump_message(&expected).unwrap()
    else {
        panic!("a jump is an endpoint control");
    };
    assert_eq!(kind, CLIENT_JUMP_KIND);
    assert_eq!(kind, "endpoint.client-jump.v1");
    assert_eq!(
        serde_json::from_str::<EndpointClientJump>(&data).unwrap(),
        expected
    );
}

#[test]
fn client_jump_resolve_fixture_lists_every_outcome() {
    let expected = [
        ClientJumpOutcome::Focused,
        ClientJumpOutcome::StaleBoot,
        ClientJumpOutcome::NotFound,
        ClientJumpOutcome::Unavailable,
        ClientJumpOutcome::Timeout,
        ClientJumpOutcome::Failed {
            message: "activation was cancelled".into(),
        },
    ]
    .into_iter()
    .enumerate()
    .map(|(index, outcome)| Request {
        id: format!("client-shell:{}", index + 8),
        method: Method::ClientJumpResolve(ClientJumpResolveParams {
            jump_id: "jump-3".into(),
            outcome,
        }),
    })
    .collect::<Vec<_>>();
    assert_canonical(fixture!("endpoint-client-jump-resolve-v1.json"), &expected);
}

#[test]
fn agent_mark_unread_fixture_is_the_request() {
    let expected = Request {
        id: "client-shell:14".into(),
        method: Method::AgentMarkUnread(PaneTarget {
            pane_id: "w1:p1".into(),
        }),
    };
    assert_canonical(fixture!("endpoint-agent-mark-unread-v1.json"), &expected);
}

fn reporting_client() -> ClientInfo {
    ClientInfo {
        client_id: 7,
        report: Some(report()),
    }
}

#[test]
fn client_list_fixture_is_the_api_response() {
    let expected = SuccessResponse {
        id: "req_clients".into(),
        result: ResponseResult::ClientList {
            boot_id: "boot-a".into(),
            clients: vec![
                reporting_client(),
                // A connection that has not reported, such as a second
                // connection of the same window.
                ClientInfo {
                    client_id: 8,
                    report: None,
                },
            ],
        },
    };
    assert_canonical(fixture!("api-client-list-v1.json"), &expected);
}

#[test]
fn client_changed_fixture_is_the_pushed_event() {
    let changed = |client_id, client| EventEnvelope {
        event: EventKind::ClientChanged,
        data: EventData::ClientChanged { client_id, client },
    };
    let expected = vec![
        changed(
            8,
            Some(ClientInfo {
                client_id: 8,
                report: None,
            }),
        ),
        changed(7, Some(reporting_client())),
        changed(7, None),
    ];
    assert_canonical(fixture!("api-client-changed-v1.json"), &expected);
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct JumpExchange {
    request: Request,
    response: SuccessResponse,
}

#[test]
fn client_jump_api_fixture_is_the_request_and_its_success() {
    let expected = JumpExchange {
        request: Request {
            id: "req_jump".into(),
            method: Method::ClientJump(ClientJumpParams {
                client_id: 7,
                endpoint_id: "ssh:0123456789abcdef0123456789abcdef".into(),
                boot_id: "boot-b".into(),
                pane_id: "w1:p4".into(),
            }),
        },
        response: SuccessResponse {
            id: "req_jump".into(),
            result: ResponseResult::ClientJumped {
                client_id: 7,
                endpoint_id: "ssh:0123456789abcdef0123456789abcdef".into(),
                pane_id: "w1:p4".into(),
            },
        },
    };
    assert_canonical(fixture!("api-client-jump-v1.json"), &expected);
}
