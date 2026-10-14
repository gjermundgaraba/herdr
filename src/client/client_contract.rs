//! Client-contract traffic that bypasses the command lane: this client's
//! answers to daemons' pickers (`ui.pick.resolve`).
//!
//! A daemon handles an answer whatever the client presents and whatever
//! command it waits on, so answers are written straight to their connection
//! instead of queueing behind the user's commands, which need the active
//! surface and wait for one another. An answer also goes to a daemon the
//! client no longer presents. The shell decides what to say
//! (`shell/picks.rs`); this module only says it, at most once per client-loop
//! turn.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/client_contract.rs` (see "Sibling fork: Herdr GPUI"
//! in `FORK.md`).

use tracing::{debug, warn};

use super::endpoint::{ClientEndpointId, EndpointControlMessage, EndpointRegistry};
use super::frame_output::ComposedFrame;
use super::shell::{ClientShellInput, ClientShellState, ControlOrigin, ResolvedPick};
use crate::api::schema::{Method, Request, UiPickOutcome, UiPickResolveParams};
use crate::protocol::endpoint::{self, EndpointPickClose, EndpointPickOpen};
use crate::protocol::ClientMessage;
use crate::server::client_commands::MAX_ENDPOINT_COMMAND_BYTES;

const PICK_REQUEST_PREFIX: &str = "client-pick:";

#[derive(Default)]
pub(super) struct ClientContract {
    next_request: u64,
}

impl ClientContract {
    /// Whether a response answers a request this module sent. Those responses
    /// never reach the command lane.
    pub(super) fn owns_response(request_id: &str) -> bool {
        request_id.starts_with(PICK_REQUEST_PREFIX)
    }

    /// Notes a daemon's rejection. Nothing is retried: a picker is answered
    /// once.
    pub(super) fn receive_response(request_id: &str, response: &[u8]) {
        let Err(error) = super::endpoint_commands::parse_response(request_id, response) else {
            return;
        };
        let code = error.code.as_deref();
        if answer_crossed_its_end(request_id, code) {
            debug!(%request_id, "the daemon had already ended what this answers");
            return;
        }
        warn!(
            %request_id,
            code = code.unwrap_or("invalid_response"),
            message = %error.message,
            "daemon rejected a client contract request"
        );
    }

    /// Runs once per client-loop turn: answers settled pickers.
    pub(super) fn sync(&mut self, shell: &mut ClientShellState, endpoints: &mut EndpointRegistry) {
        for ResolvedPick {
            origin,
            pick_id,
            outcome,
        } in shell.settle_picks()
        {
            let method = Method::UiPickResolve(UiPickResolveParams { pick_id, outcome });
            self.answer(shell, endpoints, &origin, method);
        }
    }

    /// Answers a daemon's control message on the connection it arrived on.
    /// If that connection is gone, its daemon has already cancelled the
    /// picker.
    fn answer(
        &mut self,
        shell: &ClientShellState,
        endpoints: &mut EndpointRegistry,
        origin: &ControlOrigin,
        method: Method,
    ) {
        if !endpoints.accepts(&origin.endpoint_id, origin.generation) {
            return;
        }
        let Some((boot_id, _)) =
            shell.endpoint_snapshot_identity(&origin.endpoint_id, origin.generation)
        else {
            debug!(?method, "no boot identity to answer a daemon with");
            return;
        };
        let boot_id = boot_id.to_owned();
        let request = self.request(method);
        let Some(encoded) = encode(&request) else {
            return;
        };
        // A daemon closes a connection that sends a larger command than it
        // allows. The shell keeps a picker's answer within one command
        // (`pick_answer_fits`).
        if !fits(&encoded) {
            warn!(id = %request.id, "answer exceeds the endpoint command size; not sent");
            return;
        }
        send(endpoints, &origin.endpoint_id, boot_id, encoded);
    }

    fn request(&mut self, method: Method) -> Request {
        self.next_request += 1;
        Request {
            id: format!("{PICK_REQUEST_PREFIX}{}", self.next_request),
            method,
        }
    }
}

/// The controls a daemon sends this client under the client contract.
#[derive(Debug)]
pub(crate) enum Control {
    PickOpen(EndpointPickOpen),
    PickClose(EndpointPickClose),
}

/// Decodes a contract control. `None` means the kind is not one; a control
/// whose data does not decode is ignored, as unknown controls are.
pub(super) fn decode_control(kind: &str, data: &str) -> Option<EndpointControlMessage> {
    let control = match kind {
        endpoint::PICK_OPEN_KIND => serde_json::from_str(data).map(Control::PickOpen),
        endpoint::PICK_CLOSE_KIND => serde_json::from_str(data).map(Control::PickClose),
        _ => return None,
    };
    Some(control.map_or(
        EndpointControlMessage::Ignored,
        EndpointControlMessage::Contract,
    ))
}

/// Shows or closes a picker for the endpoint connection that sent the
/// control. Returns the input outcome and, when it repaints, the frame.
pub(super) fn handle_control(
    shell: &mut ClientShellState,
    endpoint_id: &ClientEndpointId,
    generation: u64,
    control: Control,
    (width, height): (u16, u16),
) -> (ClientShellInput, Option<ComposedFrame>) {
    let origin = ControlOrigin {
        endpoint_id: endpoint_id.clone(),
        generation,
    };
    let outcome = ClientShellInput {
        repaint: match control {
            Control::PickOpen(open) => shell.receive_pick_open(origin, open),
            Control::PickClose(close) => shell.receive_pick_close(&origin, close),
        },
        ..Default::default()
    };
    let frame = outcome
        .repaint
        .then(|| shell.compose(width, height))
        .flatten();
    (outcome, frame)
}

/// Whether a rejection only says that this client's answer crossed the
/// daemon's own end of what it answers: a picker its plugin closed. The
/// daemon had forgotten it by the time the answer arrived, so
/// `pick_not_found` is the expected reply and nothing is wrong.
fn answer_crossed_its_end(request_id: &str, code: Option<&str>) -> bool {
    request_id.starts_with(PICK_REQUEST_PREFIX) && code == Some("pick_not_found")
}

/// Whether a picker's answer fits one command. Only created text, which is
/// the user's own, can outgrow one. The request is measured as it would be
/// sent, under the longest id this module can give it.
pub(super) fn pick_answer_fits(pick_id: &str, outcome: &UiPickOutcome) -> bool {
    let request = Request {
        id: format!("{PICK_REQUEST_PREFIX}{}", u64::MAX),
        method: Method::UiPickResolve(UiPickResolveParams {
            pick_id: pick_id.to_owned(),
            outcome: outcome.clone(),
        }),
    };
    encode(&request).is_some_and(|request| fits(&request))
}

fn encode(request: &Request) -> Option<String> {
    serde_json::to_string(request)
        .inspect_err(
            |error| warn!(%error, id = %request.id, "could not encode a client contract request"),
        )
        .ok()
}

fn fits(request: &str) -> bool {
    request.len() <= MAX_ENDPOINT_COMMAND_BYTES
}

/// Writes one request to its connection. A failed write is recorded by the
/// registry, which the client loop turns into a reconnect.
fn send(
    endpoints: &mut EndpointRegistry,
    endpoint_id: &ClientEndpointId,
    boot_id: String,
    request: String,
) {
    endpoints.send_to(
        endpoint_id,
        &ClientMessage::ClientShellEndpointRequest { boot_id, request },
    );
}

#[cfg(test)]
mod tests;
