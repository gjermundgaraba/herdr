//! The fixtures for the wire shapes this fork adds between the daemon and its
//! clients. Herdr GPUI's protocol crate keeps verbatim copies of these files,
//! so each test pins a fixture to the exact value the daemon decodes and to
//! its canonical encoding.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::endpoint::{
    pick_close_message, pick_open_message, EndpointPickClose, EndpointPickOpen, PICK_CLOSE_KIND,
    PICK_OPEN_KIND,
};
use super::{
    ClientShellAgent, ClientShellCommand, ClientShellCommandAction, ClientShellPane,
    ClientShellSnapshot, ClientShellTab, ClientShellWorkspace, ServerMessage,
};
use crate::agent_priority::priority_key;
use crate::api::schema::{
    AgentStatus, Method, PaneTarget, Request, UiPickCreate, UiPickItem, UiPickOutcome,
    UiPickResolveParams,
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
    }
}

#[test]
fn snapshot_fixture_carries_agent_priority() {
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
    for agent in json["agents"].as_array_mut().unwrap() {
        agent.as_object_mut().unwrap().remove("priority_key");
        agent.as_object_mut().unwrap().remove("priority_mark");
    }
    let decoded: ClientShellSnapshot = serde_json::from_value(json).unwrap();
    assert!(decoded
        .agents
        .iter()
        .all(|agent| agent.priority_key.is_empty() && agent.priority_mark.is_none()));
}

fn pick_open() -> EndpointPickOpen {
    EndpointPickOpen {
        pick_id: "pick-7".into(),
        title: "Move pane to".into(),
        items: vec![
            UiPickItem {
                id: "t1".into(),
                label: "shell".into(),
                detail: None,
                badge: None,
            },
            UiPickItem {
                id: "t2".into(),
                label: "build".into(),
                detail: Some("3 panes".into()),
                badge: Some("2".into()),
            },
        ],
        selected: Some("t2".into()),
        create: Some(UiPickCreate {
            label: "New tab".into(),
        }),
    }
}

#[test]
fn pick_open_fixture_is_the_control_payload() {
    let expected = pick_open();
    assert_canonical(fixture!("endpoint-pick-open-v1.json"), &expected);

    let ServerMessage::EndpointControl { kind, data } = pick_open_message(&expected).unwrap()
    else {
        panic!("opening a picker is an endpoint control");
    };
    assert_eq!(kind, PICK_OPEN_KIND);
    assert_eq!(kind, "endpoint.pick.open.v1");
    assert_eq!(
        serde_json::from_str::<EndpointPickOpen>(&data).unwrap(),
        expected
    );
}

#[test]
fn pick_close_fixture_is_the_control_payload() {
    let expected = EndpointPickClose {
        pick_id: "pick-7".into(),
    };
    assert_canonical(fixture!("endpoint-pick-close-v1.json"), &expected);

    let ServerMessage::EndpointControl { kind, data } = pick_close_message("pick-7").unwrap()
    else {
        panic!("closing a picker is an endpoint control");
    };
    assert_eq!(kind, PICK_CLOSE_KIND);
    assert_eq!(kind, "endpoint.pick.close.v1");
    assert_eq!(
        serde_json::from_str::<EndpointPickClose>(&data).unwrap(),
        expected
    );
}

#[test]
fn pick_resolve_fixture_lists_every_outcome() {
    let expected = [
        UiPickOutcome::Picked { id: "t2".into() },
        UiPickOutcome::Created {
            text: "New tab name".into(),
        },
        UiPickOutcome::Cancelled,
    ]
    .map(|outcome| UiPickResolveParams {
        pick_id: "pick-7".into(),
        outcome,
    });
    assert_canonical(
        fixture!("endpoint-pick-resolve-v1.json"),
        &expected.to_vec(),
    );

    // The params are what the client sends on its command lane.
    let request = serde_json::to_value(Request {
        id: "client-shell:1".into(),
        method: Method::UiPickResolve(expected[0].clone()),
    })
    .unwrap();
    assert_eq!(request["method"], "ui.pick.resolve");
    assert_eq!(
        request["params"],
        serde_json::from_str::<serde_json::Value>(fixture!("endpoint-pick-resolve-v1.json"))
            .unwrap()[0]
    );
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
