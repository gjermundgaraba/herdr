use super::client_contract::lane;
use super::*;
use crate::api::schema::{
    Method, Request, ResponseResult, UiPickItem, UiPickOutcome, UiPickParams,
};
use crate::protocol::endpoint::{
    EndpointPickClose, EndpointPickOpen, PICK_CLOSE_KIND, PICK_OPEN_KIND,
};

fn request_pick(
    server: &mut HeadlessServer,
    pick_id: &str,
    client_id: Option<u64>,
) -> std::sync::mpsc::Receiver<String> {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: Request {
            id: pick_id.into(),
            method: Method::UiPick(UiPickParams {
                title: "Move pane".into(),
                items: vec![UiPickItem {
                    id: "w1:t2".into(),
                    label: "2 build".into(),
                    detail: None,
                    badge: None,
                }],
                selected: None,
                create: None,
                client_id,
            }),
        },
        respond_to,
        response_write_complete: None,
    });
    response_rx
}

pub(super) fn next_control(
    receiver: &std::sync::mpsc::Receiver<Vec<u8>>,
    expected: &str,
) -> String {
    loop {
        let message = read_server_message(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("expected {expected}")),
        );
        if let ServerMessage::EndpointControl { kind, data } = message {
            if kind == expected {
                return data;
            }
        }
    }
}

fn outcome(response: &str) -> UiPickOutcome {
    let response: api::schema::SuccessResponse = serde_json::from_str(response).expect(response);
    let ResponseResult::UiPicked { outcome } = response.result else {
        panic!("expected a pick outcome: {response:?}");
    };
    outcome
}

pub(super) fn error_code(response: &str) -> String {
    let response: api::schema::ErrorResponse = serde_json::from_str(response).expect(response);
    response.error.code
}

fn resolve(pick_id: &str, outcome: UiPickOutcome) -> Method {
    Method::UiPickResolve(api::schema::UiPickResolveParams {
        pick_id: pick_id.into(),
        outcome,
    })
}

#[tokio::test]
async fn only_the_showing_client_resolves_a_pick() {
    let mut server = test_headless_server();
    let (first, _) = connect_matching_test_shell(&mut server, 61);
    let (second, _) = connect_matching_test_shell(&mut server, 62);
    let waiting = request_pick(&mut server, "pick-1", Some(62));
    let open: EndpointPickOpen =
        serde_json::from_str(&next_control(&second, PICK_OPEN_KIND)).unwrap();
    assert_eq!(open.pick_id, "pick-1");
    assert_eq!(open.items[0].id, "w1:t2");
    assert!(first.try_iter().all(|bytes| !matches!(
        read_server_message(bytes),
        ServerMessage::EndpointControl { kind, .. } if kind == PICK_OPEN_KIND
    )));
    let picked = UiPickOutcome::Picked { id: "w1:t2".into() };

    let wrong = lane(
        &mut server,
        &first,
        61,
        "test-boot",
        resolve("pick-1", picked.clone()),
    );
    assert_eq!(error_code(&wrong), "pick_not_found");
    assert!(waiting.try_recv().is_err());

    lane(
        &mut server,
        &second,
        62,
        "test-boot",
        resolve("pick-1", picked.clone()),
    );
    assert_eq!(outcome(&waiting.try_recv().unwrap()), picked);
    assert!(server.fork.pending_picks.is_empty());
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_answers_its_pick_whatever_it_presents_and_however_busy_it_is() {
    let mut server = test_headless_server();
    let (control, _) = connect_matching_test_shell(&mut server, 61);
    let waiting = request_pick(&mut server, "pick-1", Some(61));
    next_control(&control, PICK_OPEN_KIND);
    // The client switched to another endpoint, and a command is still running.
    server.set_client_shell_surface_active(61, false).unwrap();
    server
        .clients
        .get_mut(&61)
        .unwrap()
        .shell_endpoint_command_in_flight = true;

    let answered = lane(
        &mut server,
        &control,
        61,
        "test-boot",
        resolve("pick-1", UiPickOutcome::Cancelled),
    );
    assert!(answered.contains(r#""type":"ok""#), "{answered}");
    assert_eq!(
        outcome(&waiting.try_recv().unwrap()),
        UiPickOutcome::Cancelled
    );
    assert!(server.clients[&61].shell_endpoint_command_in_flight);

    // Pick answers come from a client connection only.
    let (respond_to, response) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: Request {
            id: "api".into(),
            method: resolve("pick-1", UiPickOutcome::Cancelled),
        },
        respond_to,
        response_write_complete: None,
    });
    assert_eq!(
        error_code(&response.try_recv().unwrap()),
        "connection_local_only"
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_client_shows_one_pick_and_closing_cancels_it() {
    let mut server = test_headless_server();
    let (control, _) = connect_matching_test_shell(&mut server, 61);
    server.foreground_client_id = Some(61);
    // A stale client id falls back to the foreground client.
    let waiting = request_pick(&mut server, "pick-1", Some(999));
    next_control(&control, PICK_OPEN_KIND);
    let busy = request_pick(&mut server, "pick-2", None);
    assert_eq!(error_code(&busy.try_recv().unwrap()), "ui_busy");

    let (respond_to, closed) = std::sync::mpsc::channel();
    server.handle_api_request_with_shutdown_check(api::ApiRequestMessage {
        request: Request {
            id: "close".into(),
            method: Method::UiPickClose(api::schema::UiPickCloseParams {
                pick_id: "pick-1".into(),
            }),
        },
        respond_to,
        response_write_complete: None,
    });
    assert!(closed.try_recv().unwrap().contains("\"ok\""));
    assert_eq!(
        outcome(&waiting.try_recv().unwrap()),
        UiPickOutcome::Cancelled
    );
    let close: EndpointPickClose =
        serde_json::from_str(&next_control(&control, PICK_CLOSE_KIND)).unwrap();
    assert_eq!(close.pick_id, "pick-1");
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn a_departing_client_cancels_its_pick_and_no_client_is_an_error() {
    let mut server = test_headless_server();
    let none = request_pick(&mut server, "pick-0", None);
    assert_eq!(error_code(&none.try_recv().unwrap()), "no_client");

    let (_control, _) = connect_matching_test_shell(&mut server, 61);
    let waiting = request_pick(&mut server, "pick-1", Some(61));
    server.remove_client(61);
    assert_eq!(
        outcome(&waiting.try_recv().unwrap()),
        UiPickOutcome::Cancelled
    );
    shutdown_test_runtimes(&mut server);
}
