//! The client loop's side of the contract: what `ClientContract::sync` writes
//! to which connection, turn by turn. Transports record instead of sending.

use std::sync::{Arc, Mutex};

use super::*;
use crate::api::schema::UiPickOutcome;
use crate::client::endpoint::{EndpointNegotiation, EndpointTransport};
use crate::client::shell::tests::fleet::{fleet, machine};
use crate::client::shell::ControlOrigin;
use crate::protocol::endpoint::EndpointPickOpen;

type Sent = Arc<Mutex<Vec<ClientMessage>>>;

struct Recorder(Sent);

impl EndpointTransport for Recorder {
    fn send(&mut self, message: &ClientMessage) -> std::io::Result<()> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}

/// The lane requests a connection carried, as `(boot id, request)`.
fn requests(sent: &Sent) -> Vec<(String, Request)> {
    sent.lock()
        .unwrap()
        .iter()
        .map(|message| match message {
            ClientMessage::ClientShellEndpointRequest { boot_id, request } => (
                boot_id.clone(),
                serde_json::from_str(request).expect("lane request"),
            ),
            other => panic!("unexpected message: {other:?}"),
        })
        .collect()
}

/// A client attached to Local and Build, both connected, presenting Local.
struct Client {
    shell: ClientShellState,
    endpoints: EndpointRegistry,
    contract: ClientContract,
    remote: ClientEndpointId,
    local_sent: Sent,
    remote_sent: Sent,
}

impl Client {
    fn new() -> Self {
        let (mut shell, remotes) = fleet(&["Build"]);
        shell.set_snapshot(Box::new(machine("boot-1", Vec::new())));
        shell.set_endpoint_snapshot(&remotes[0], Box::new(machine("build-boot", Vec::new())));
        let local_sent = Sent::default();
        let remote_sent = Sent::default();
        let mut endpoints = EndpointRegistry::new(
            Recorder(local_sent.clone()),
            1,
            EndpointNegotiation::default(),
        );
        endpoints.insert(
            remotes[0].clone(),
            Recorder(remote_sent.clone()),
            2,
            EndpointNegotiation::default(),
            false,
        );
        Self {
            shell,
            endpoints,
            contract: ClientContract::default(),
            remote: remotes[0].clone(),
            local_sent,
            remote_sent,
        }
    }

    /// One client-loop turn.
    fn turn(&mut self) {
        self.contract.sync(&mut self.shell, &mut self.endpoints);
    }

    /// Local's plugin opens a picker on this client.
    fn open_local_pick(&mut self) {
        assert!(self.shell.receive_pick_open(local(), pick_open()));
    }
}

/// The connection Local's daemon sends its controls on.
fn local() -> ControlOrigin {
    ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    }
}

fn pick_open() -> EndpointPickOpen {
    EndpointPickOpen {
        pick_id: "pick-1".into(),
        title: "Move pane".into(),
        items: Vec::new(),
        selected: None,
        create: None,
    }
}

/// The picker answers a connection carried, as `(boot id, pick id, outcome)`.
fn pick_answers(sent: &Sent) -> Vec<(String, String, UiPickOutcome)> {
    requests(sent)
        .into_iter()
        .filter_map(|(boot_id, request)| match request.method {
            Method::UiPickResolve(params) => {
                assert!(ClientContract::owns_response(&request.id), "{}", request.id);
                Some((boot_id, params.pick_id, params.outcome))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn an_answer_is_dropped_when_its_connection_is_gone() {
    let mut client = Client::new();
    client.open_local_pick();
    client.shell.handle_input_bytes(b"\x1b");

    // Local reconnected before the turn: the old daemon connection, and the
    // picker it was waiting on, are gone.
    let reconnected = Sent::default();
    client.endpoints.insert(
        ClientEndpointId::Local,
        Recorder(reconnected.clone()),
        7,
        EndpointNegotiation::default(),
        true,
    );
    client.turn();

    assert!(pick_answers(&client.local_sent).is_empty());
    assert!(pick_answers(&reconnected).is_empty());
}

#[test]
fn the_daemons_reply_to_a_picker_answer_that_crossed_its_close_is_expected() {
    let mut client = Client::new();
    client.open_local_pick();
    client.shell.handle_input_bytes(b"\x1b");
    client.turn();
    let sent = requests(&client.local_sent);
    let answer_id = sent
        .iter()
        .find(|(_, request)| matches!(request.method, Method::UiPickResolve(_)))
        .map(|(_, request)| request.id.as_str())
        .expect("the picker's answer");

    // The plugin closed the picker while the answer was on its way.
    assert!(ClientContract::owns_response(answer_id));
    assert!(answer_crossed_its_end(Some("pick_not_found")));
    // Any other rejection is not.
    assert!(!answer_crossed_its_end(Some("stale_boot")));
    assert!(!answer_crossed_its_end(None));
}

#[test]
fn a_picker_answer_is_measured_as_the_request_that_carries_it() {
    let created = |len: usize| UiPickOutcome::Created {
        text: "x".repeat(len),
    };
    let envelope = serde_json::to_string(&Request {
        id: format!("{PICK_REQUEST_PREFIX}{}", u64::MAX),
        method: Method::UiPickResolve(UiPickResolveParams {
            pick_id: "pick-1".into(),
            outcome: created(0),
        }),
    })
    .unwrap()
    .len();
    let room = MAX_ENDPOINT_COMMAND_BYTES - envelope;
    assert!(pick_answer_fits("pick-1", &created(room)));
    assert!(!pick_answer_fits("pick-1", &created(room + 1)));
    assert!(pick_answer_fits("pick-1", &UiPickOutcome::Cancelled));
}

#[test]
fn a_switch_to_another_endpoint_cancels_the_open_picker_on_its_own_connection() {
    let mut client = Client::new();
    client.open_local_pick();
    client.turn();
    assert!(pick_answers(&client.local_sent).is_empty(), "still open");

    // The user switches to Build while Local's plugin waits.
    let remote = client.remote.clone();
    assert!(client.shell.activate_endpoint_projection(&remote));
    client.turn();

    // Local's daemon, which opened the picker, hears that it closed, though
    // the client now presents Build.
    assert_eq!(
        pick_answers(&client.local_sent),
        [(
            "boot-1".to_owned(),
            "pick-1".to_owned(),
            UiPickOutcome::Cancelled
        )]
    );
    assert!(pick_answers(&client.remote_sent).is_empty());
    client.turn();
    assert_eq!(pick_answers(&client.local_sent).len(), 1, "answered once");
}
