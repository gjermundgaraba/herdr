//! `client.report`, `client.list`, `client.changed`, and `client.jump`.

use super::picks::{error_code, next_control};
use super::*;
use crate::api::schema::{
    AgentStatus, ClientEndpointStatus, ClientInfo, ClientJumpOutcome, ClientJumpParams,
    ClientJumpResolveParams, ClientReport, ClientReportAgent, ClientReportEndpoint, EmptyParams,
    EventData, Method, Request, ResponseResult,
};
use crate::protocol::endpoint::{EndpointClientJump, CLIENT_JUMP_KIND};
use crate::protocol::ClientShellAgent;

fn report(pane_ids: &[&str]) -> ClientReport {
    ClientReport {
        focused: Some(true),
        active_endpoint_id: Some("local".into()),
        endpoints: vec![ClientReportEndpoint {
            endpoint_id: "local".into(),
            label: "Local".into(),
            status: ClientEndpointStatus::Online,
            boot_id: Some("test-boot".into()),
        }],
        agents: pane_ids
            .iter()
            .enumerate()
            .map(|(index, pane_id)| ClientReportAgent {
                endpoint_id: "local".into(),
                focused: index == 0,
                workspace_label: "api".into(),
                workspace_number: 1,
                cwd: Some("/work/api".into()),
                foreground_cwd: None,
                agent: ClientShellAgent {
                    pane_id: (*pane_id).into(),
                    workspace_id: "w1".into(),
                    tab_id: "w1:t1".into(),
                    name: None,
                    display_agent: None,
                    agent: Some("claude".into()),
                    title: None,
                    terminal_title: None,
                    terminal_title_stripped: None,
                    agent_status: AgentStatus::Done,
                    state_change_seq: 4,
                    state_labels: Vec::new(),
                    tokens: Vec::new(),
                    focused: index == 0,
                    priority_key: "1.1.0000000000000004".into(),
                    priority_mark: None,
                },
            })
            .collect(),
    }
}

/// Sends one request down a client's command lane and returns its response.
pub(super) fn lane(
    server: &mut HeadlessServer,
    control: &std::sync::mpsc::Receiver<Vec<u8>>,
    client_id: u64,
    boot_id: &str,
    method: Method,
) -> String {
    server.handle_server_event(ServerEvent::ClientShellEndpointRequest {
        client_id,
        boot_id: boot_id.into(),
        request: Box::new(Request {
            id: "lane".into(),
            method,
        }),
    });
    loop {
        let message = read_server_message(
            control
                .recv_timeout(Duration::from_secs(1))
                .expect("lane response"),
        );
        if let ServerMessage::ClientShellEndpointResponseChunk {
            request_id,
            final_chunk,
            data,
            ..
        } = message
        {
            assert_eq!(request_id, "lane");
            assert!(final_chunk);
            return String::from_utf8(data).expect("response text");
        }
    }
}

fn api(server: &mut HeadlessServer, method: Method) -> std::sync::mpsc::Receiver<String> {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: Request {
            id: "api".into(),
            method,
        },
        respond_to,
        response_write_complete: None,
    });
    response_rx
}

fn listed(server: &mut HeadlessServer) -> Vec<ClientInfo> {
    let response = api(server, Method::ClientList(EmptyParams::default()))
        .try_recv()
        .expect("client.list response");
    let response: api::schema::SuccessResponse = serde_json::from_str(&response).expect(&response);
    let ResponseResult::ClientList { boot_id, clients } = response.result else {
        panic!("expected a client list: {response:?}");
    };
    assert_eq!(boot_id, "test-boot");
    clients
}

/// The client ids of the `client.changed` events pushed after `sequence`.
fn changes(hub: &api::EventHub, sequence: u64) -> Vec<u64> {
    hub.events_after(sequence)
        .into_iter()
        .filter_map(|(_, event)| match event.data {
            EventData::ClientChanged { client_id } => Some(client_id),
            _ => None,
        })
        .collect()
}

/// Attaches a client shell that has sent a report, as a jump target must.
fn reporting_shell(
    server: &mut HeadlessServer,
    client_id: u64,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (control, _) = connect_matching_test_shell(server, client_id);
    let response = lane(
        server,
        &control,
        client_id,
        "test-boot",
        Method::ClientReport(report(&["w1:p1"])),
    );
    assert!(response.contains(r#""type":"ok""#), "{response}");
    control
}

fn jump(server: &mut HeadlessServer, client_id: u64) -> std::sync::mpsc::Receiver<String> {
    api(
        server,
        Method::ClientJump(ClientJumpParams {
            client_id,
            endpoint_id: "ssh:build".into(),
            boot_id: "remote-boot".into(),
            pane_id: "w2:p3".into(),
        }),
    )
}

fn resolve(jump_id: &str, outcome: ClientJumpOutcome) -> Method {
    Method::ClientJumpResolve(ClientJumpResolveParams {
        jump_id: jump_id.into(),
        outcome,
    })
}

fn forwarded_jump(control: &std::sync::mpsc::Receiver<Vec<u8>>) -> EndpointClientJump {
    serde_json::from_str(&next_control(control, CLIENT_JUMP_KIND)).unwrap()
}

#[tokio::test]
async fn a_report_is_listed_announced_and_dropped_with_its_client() {
    let hub = api::EventHub::default();
    let mut server = test_headless_server_with_event_hub(hub.clone());
    let (first, _) = connect_matching_test_shell(&mut server, 61);
    let (_second, _) = connect_matching_test_shell(&mut server, 62);
    let bare = |client_id| ClientInfo {
        client_id,
        report: None,
    };
    assert_eq!(listed(&mut server), [bare(61), bare(62)]);
    assert_eq!(changes(&hub, 0), [61, 62]);

    let before = hub.current_sequence();
    let sent = report(&["w1:p1", "w1:p2"]);
    let response = lane(
        &mut server,
        &first,
        61,
        "test-boot",
        Method::ClientReport(sent.clone()),
    );
    assert!(response.contains(r#""type":"ok""#), "{response}");
    let reported = ClientInfo {
        client_id: 61,
        report: Some(sent.clone()),
    };
    assert_eq!(listed(&mut server), [reported, bare(62)]);
    assert_eq!(changes(&hub, before), [61]);

    // The same report again changes nothing, so nobody is told.
    let before = hub.current_sequence();
    lane(
        &mut server,
        &first,
        61,
        "test-boot",
        Method::ClientReport(sent),
    );
    assert!(changes(&hub, before).is_empty());

    // A newer report replaces the stored one.
    let newer = report(&["w1:p2"]);
    lane(
        &mut server,
        &first,
        61,
        "test-boot",
        Method::ClientReport(newer.clone()),
    );
    assert_eq!(server.client_reports.get(61), Some(&newer));
    assert_eq!(changes(&hub, before).len(), 1);

    let before = hub.current_sequence();
    server.remove_client(61);
    assert_eq!(server.client_reports.get(61), None);
    assert_eq!(listed(&mut server), [bare(62)]);
    assert_eq!(changes(&hub, before), [61]);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_reports_whatever_it_presents_and_however_busy_it_is() {
    let mut server = test_headless_server();
    let (first, _) = connect_matching_test_shell(&mut server, 61);
    let (_second, _) = connect_matching_test_shell(&mut server, 62);
    assert_eq!(server.foreground_client_id, Some(62));
    // The client presents another endpoint, and a command is still running.
    server.set_client_shell_surface_active(61, false).unwrap();
    server
        .clients
        .get_mut(&61)
        .unwrap()
        .shell_endpoint_command_in_flight = true;

    let sent = report(&["w1:p1"]);
    let response = lane(
        &mut server,
        &first,
        61,
        "test-boot",
        Method::ClientReport(sent.clone()),
    );
    assert!(response.contains(r#""type":"ok""#), "{response}");
    assert_eq!(server.client_reports.get(61), Some(&sent));
    assert!(server.clients[&61].shell_endpoint_command_in_flight);
    // Reporting is not user activity: it does not take over the foreground.
    assert_eq!(server.foreground_client_id, Some(62));

    let mut malformed = report(&["w1:p1"]);
    malformed.agents[0].endpoint_id = "ssh:gone".into();
    let response = lane(
        &mut server,
        &first,
        61,
        "test-boot",
        Method::ClientReport(malformed),
    );
    assert_eq!(error_code(&response), "invalid_params");
    assert_eq!(server.client_reports.get(61), Some(&sent));

    let response = lane(
        &mut server,
        &first,
        61,
        "earlier-boot",
        Method::ClientReport(report(&[])),
    );
    assert_eq!(error_code(&response), "stale_boot");

    // Reports and jump answers come from a client connection only.
    for method in [
        Method::ClientReport(report(&[])),
        resolve("jump-1", ClientJumpOutcome::Focused),
    ] {
        let response = api(&mut server, method).try_recv().unwrap();
        assert_eq!(error_code(&response), "connection_local_only");
    }
    assert_eq!(server.client_reports.get(61), Some(&sent));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_request_still_renders_the_metadata_that_expired_with_it() {
    let mut server = test_headless_server();
    let workspace = crate::workspace::Workspace::test_new("metadata");
    let pane_id = workspace.tabs[0].root_pane;
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    let list = |server: &mut HeadlessServer| {
        let (respond_to, response_rx) = std::sync::mpsc::channel();
        let render = server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
            request: Request {
                id: "api".into(),
                method: Method::ClientList(EmptyParams::default()),
            },
            respond_to,
            response_write_complete: None,
        });
        assert!(response_rx.try_recv().is_ok());
        render
    };
    let title = |server: &HeadlessServer| {
        let terminal_id = &server.app.state.workspaces[0]
            .pane_state(pane_id)
            .expect("pane")
            .attached_terminal_id;
        server.app.state.terminals[terminal_id].effective_title()
    };
    // Nothing is due: listing clients changes nothing a client draws.
    assert!(!list(&mut server));

    server.handle_internal_event_with_forwarding(AppEvent::HookStateReported {
        pane_id,
        source: "custom:pi".into(),
        agent_label: "pi".into(),
        state: crate::detect::AgentState::Working,
        message: None,
        seq: None,
        session_ref: None,
    });
    server.handle_internal_event_with_forwarding(AppEvent::HookMetadataReported {
        pane_id,
        source: "user:pi-display".into(),
        agent_label: Some("pi".into()),
        applies_to_source: Some("custom:pi".into()),
        title: Some("short lived".into()),
        display_agent: None,
        state_labels: HashMap::new(),
        clear_title: false,
        clear_display_agent: false,
        clear_state_labels: false,
        seq: None,
        ttl: Some(Duration::from_millis(1)),
    });
    let deadline = server
        .app
        .agent_metadata_deadline
        .expect("metadata deadline");
    while Instant::now() < deadline {
        std::thread::yield_now();
    }

    // The title is due when the listing is taken up, and that listing is the
    // only thing left to ask for the render.
    assert!(list(&mut server));
    assert_eq!(title(&server), None);
    assert!(!list(&mut server));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_jump_reaches_one_client_and_returns_its_outcome() {
    let mut server = test_headless_server();
    let first = reporting_shell(&mut server, 61);
    let second = reporting_shell(&mut server, 62);
    // The client may be presenting another endpoint when the jump arrives.
    server.set_client_shell_surface_active(62, false).unwrap();

    let waiting = jump(&mut server, 62);
    assert_eq!(
        forwarded_jump(&second),
        EndpointClientJump {
            jump_id: "jump-1".into(),
            endpoint_id: "ssh:build".into(),
            boot_id: "remote-boot".into(),
            pane_id: "w2:p3".into(),
        }
    );
    assert!(first.try_iter().all(|bytes| !matches!(
        read_server_message(bytes),
        ServerMessage::EndpointControl { kind, .. } if kind == CLIENT_JUMP_KIND
    )));

    let wrong = lane(
        &mut server,
        &first,
        61,
        "test-boot",
        resolve("jump-1", ClientJumpOutcome::Focused),
    );
    assert_eq!(error_code(&wrong), "jump_not_found");
    assert!(waiting.try_recv().is_err());

    let answered = lane(
        &mut server,
        &second,
        62,
        "test-boot",
        resolve("jump-1", ClientJumpOutcome::Focused),
    );
    assert!(answered.contains(r#""type":"ok""#), "{answered}");
    let response: api::schema::SuccessResponse =
        serde_json::from_str(&waiting.try_recv().unwrap()).unwrap();
    assert_eq!(response.id, "api");
    assert_eq!(
        response.result,
        ResponseResult::ClientJumped {
            client_id: 62,
            endpoint_id: "ssh:build".into(),
            pane_id: "w2:p3".into(),
        }
    );
    assert!(server.client_jumps.is_empty());

    let repeated = lane(
        &mut server,
        &second,
        62,
        "test-boot",
        resolve("jump-1", ClientJumpOutcome::Focused),
    );
    assert_eq!(error_code(&repeated), "jump_not_found");
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_newer_jump_replaces_the_one_its_client_is_performing() {
    let mut server = test_headless_server();
    let control = reporting_shell(&mut server, 61);
    let other = reporting_shell(&mut server, 62);

    let first = jump(&mut server, 61);
    assert_eq!(forwarded_jump(&control).jump_id, "jump-1");
    // Another client's jump is its own.
    let elsewhere = jump(&mut server, 62);
    assert_eq!(forwarded_jump(&other).jump_id, "jump-2");

    // The second jump goes to the client under a new id, and the first
    // caller hears at once that it lost.
    let second = jump(&mut server, 61);
    assert_eq!(forwarded_jump(&control).jump_id, "jump-3");
    let superseded: api::schema::ErrorResponse =
        serde_json::from_str(&first.try_recv().unwrap()).unwrap();
    assert_eq!(superseded.id, "api");
    assert_eq!(superseded.error.code, "jump_superseded");
    assert!(second.try_recv().is_err());
    assert!(elsewhere.try_recv().is_err());

    // The client's late answer to the replaced jump finds nothing, and does
    // not settle the jump that replaced it.
    let late = lane(
        &mut server,
        &control,
        61,
        "test-boot",
        resolve("jump-1", ClientJumpOutcome::Focused),
    );
    assert_eq!(error_code(&late), "jump_not_found");
    assert!(second.try_recv().is_err());
    assert!(first.try_recv().is_err());

    let answered = lane(
        &mut server,
        &control,
        61,
        "test-boot",
        resolve("jump-3", ClientJumpOutcome::Focused),
    );
    assert!(answered.contains(r#""type":"ok""#), "{answered}");
    let response: api::schema::SuccessResponse =
        serde_json::from_str(&second.try_recv().unwrap()).unwrap();
    assert!(
        matches!(
            response.result,
            ResponseResult::ClientJumped { client_id: 61, .. }
        ),
        "{response:?}"
    );
    // The other client's jump is still open.
    assert!(elsewhere.try_recv().is_err());
    assert!(!server.client_jumps.is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_malformed_jump_replaces_nothing() {
    let mut server = test_headless_server();
    let control = reporting_shell(&mut server, 61);
    let first = jump(&mut server, 61);
    assert_eq!(forwarded_jump(&control).jump_id, "jump-1");

    let malformed = api(
        &mut server,
        Method::ClientJump(ClientJumpParams {
            client_id: 61,
            endpoint_id: String::new(),
            boot_id: "remote-boot".into(),
            pane_id: "w2:p3".into(),
        }),
    );
    assert_eq!(error_code(&malformed.try_recv().unwrap()), "invalid_params");

    // The jump the client is performing is untouched.
    assert!(first.try_recv().is_err());
    let answered = lane(
        &mut server,
        &control,
        61,
        "test-boot",
        resolve("jump-1", ClientJumpOutcome::Focused),
    );
    assert!(answered.contains(r#""type":"ok""#), "{answered}");
    assert!(first.try_recv().is_ok());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_jump_that_cannot_be_forwarded_replaces_nothing() {
    let mut server = test_headless_server();
    let control = reporting_shell(&mut server, 61);
    let first = jump(&mut server, 61);
    assert_eq!(forwarded_jump(&control).jump_id, "jump-1");

    // The client's connection stops taking writes before the daemon has
    // noticed that it left.
    server.clients[&61].writer.as_ref().unwrap().test_close();
    let second = jump(&mut server, 61);
    assert_eq!(error_code(&second.try_recv().unwrap()), "client_departed");

    // The failed write removed the client, so the jump it was performing
    // ends as its own caller's `client_departed`, not as `jump_superseded`.
    assert_eq!(error_code(&first.try_recv().unwrap()), "client_departed");
    assert!(server.client_jumps.is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_jump_fails_when_its_client_is_unknown_or_departs() {
    let mut server = test_headless_server();
    let unknown = jump(&mut server, 61);
    assert_eq!(error_code(&unknown.try_recv().unwrap()), "client_not_found");

    // A client that has not reported is refused at once, not waited on.
    let (control, _) = connect_matching_test_shell(&mut server, 61);
    let silent = jump(&mut server, 61);
    assert_eq!(
        error_code(&silent.try_recv().unwrap()),
        "client_not_reporting"
    );
    assert!(server.client_jumps.is_empty());
    lane(
        &mut server,
        &control,
        61,
        "test-boot",
        Method::ClientReport(report(&["w1:p1"])),
    );
    let waiting = jump(&mut server, 61);
    assert!(waiting.try_recv().is_err());
    server.remove_client(61);
    assert_eq!(error_code(&waiting.try_recv().unwrap()), "client_departed");
    assert!(server.client_jumps.is_empty());

    let departed = jump(&mut server, 61);
    assert_eq!(
        error_code(&departed.try_recv().unwrap()),
        "client_not_found"
    );

    let (_control, _) = connect_matching_test_shell(&mut server, 62);
    let malformed = api(
        &mut server,
        Method::ClientJump(ClientJumpParams {
            client_id: 62,
            endpoint_id: "local".into(),
            boot_id: "test-boot".into(),
            pane_id: String::new(),
        }),
    );
    assert_eq!(error_code(&malformed.try_recv().unwrap()), "invalid_params");
    assert!(server.client_jumps.is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_reported_failure_becomes_a_typed_error() {
    let mut server = test_headless_server();
    let control = reporting_shell(&mut server, 61);
    for (outcome, code) in [
        (ClientJumpOutcome::StaleBoot, "stale_boot"),
        (ClientJumpOutcome::NotFound, "jump_target_not_found"),
        (ClientJumpOutcome::Unavailable, "jump_endpoint_unavailable"),
        (ClientJumpOutcome::Timeout, "jump_timeout"),
        (
            ClientJumpOutcome::Failed {
                message: "activation was cancelled".into(),
            },
            "jump_failed",
        ),
    ] {
        let waiting = jump(&mut server, 61);
        let jump_id = forwarded_jump(&control).jump_id;
        lane(
            &mut server,
            &control,
            61,
            "test-boot",
            resolve(&jump_id, outcome.clone()),
        );
        let response: api::schema::ErrorResponse =
            serde_json::from_str(&waiting.try_recv().unwrap()).unwrap();
        assert_eq!(response.id, "api");
        assert_eq!(response.error.code, code, "{outcome:?}");
        if matches!(outcome, ClientJumpOutcome::Failed { .. }) {
            assert_eq!(response.error.message, "activation was cancelled");
        }
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_jump_its_client_never_answers_times_out() {
    let mut server = test_headless_server();
    let control = reporting_shell(&mut server, 61);
    let waiting = jump(&mut server, 61);
    let jump_id = forwarded_jump(&control).jump_id;

    let now = Instant::now();
    server.handle_scheduled_tasks_headless(now, false);
    assert!(waiting.try_recv().is_err());
    server.handle_scheduled_tasks_headless(
        now + super::jumps::CLIENT_JUMP_TIMEOUT + Duration::from_secs(1),
        false,
    );
    assert_eq!(error_code(&waiting.try_recv().unwrap()), "jump_timeout");

    // The late answer finds nothing, and the client can jump again.
    let late = lane(
        &mut server,
        &control,
        61,
        "test-boot",
        resolve(&jump_id, ClientJumpOutcome::Focused),
    );
    assert_eq!(error_code(&late), "jump_not_found");
    let again = jump(&mut server, 61);
    assert!(again.try_recv().is_err());
    shutdown_test_runtimes(&mut server);
}
