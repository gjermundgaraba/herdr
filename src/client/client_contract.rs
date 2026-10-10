//! Client-contract traffic that bypasses the command lane: this client's
//! `client.report` to the Local daemon, and its answers to daemons' jumps
//! (`client.jump.resolve`) and pickers (`ui.pick.resolve`).
//!
//! A daemon handles both whatever the client presents and whatever command it
//! waits on, so they are written straight to their connection instead of
//! queueing behind the user's commands, which need the active surface and
//! wait for one another. An answer also goes to a daemon the client no
//! longer presents. The shell decides what to say (`shell/client_report.rs`,
//! `shell/client_jump.rs`, `shell/picks.rs`); this module only says it, at
//! most once per client-loop turn.
//!
//! Mirrored client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/client_contract.rs` (see "Sibling fork: Herdr GPUI"
//! in `FORK.md`).

use std::time::Instant;

use tracing::{debug, warn};

use super::endpoint::{ClientEndpointId, EndpointRegistry};
use super::shell::{ClientShellState, ControlOrigin, ResolvedClientJump, ResolvedPick};
use crate::api::schema::{
    ClientJumpResolveParams, ClientReport, Method, Request, UiPickOutcome, UiPickResolveParams,
};
use crate::protocol::ClientMessage;
use crate::server::client_commands::MAX_ENDPOINT_COMMAND_BYTES;

const REPORT_REQUEST_PREFIX: &str = "client-report:";
const JUMP_REQUEST_PREFIX: &str = "client-jump:";
const PICK_REQUEST_PREFIX: &str = "client-pick:";

/// The report the Local daemon holds for this client's connection to it.
struct SentReport {
    generation: u64,
    boot_id: String,
    fingerprint: u64,
    report: ClientReport,
}

#[derive(Default)]
pub(super) struct ClientContract {
    next_request: u64,
    sent: Option<SentReport>,
}

impl ClientContract {
    /// Whether a response answers a request this module sent. Those responses
    /// never reach the command lane.
    pub(super) fn owns_response(request_id: &str) -> bool {
        [
            REPORT_REQUEST_PREFIX,
            JUMP_REQUEST_PREFIX,
            PICK_REQUEST_PREFIX,
        ]
        .iter()
        .any(|prefix| request_id.starts_with(prefix))
    }

    /// Notes a daemon's rejection. Nothing is retried: the next report
    /// replaces a rejected one, and a jump or picker is answered once.
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

    /// Runs once per client-loop turn: answers settled jumps and pickers,
    /// and sends the report when it changed.
    pub(super) fn sync(
        &mut self,
        shell: &mut ClientShellState,
        endpoints: &mut EndpointRegistry,
        now: Instant,
    ) {
        for ResolvedClientJump {
            origin,
            jump_id,
            outcome,
        } in shell.settle_client_jumps(now)
        {
            let method = Method::ClientJumpResolve(ClientJumpResolveParams { jump_id, outcome });
            self.answer(shell, endpoints, &origin, JUMP_REQUEST_PREFIX, method);
        }
        for ResolvedPick {
            origin,
            pick_id,
            outcome,
        } in shell.settle_picks()
        {
            let method = Method::UiPickResolve(UiPickResolveParams { pick_id, outcome });
            self.answer(shell, endpoints, &origin, PICK_REQUEST_PREFIX, method);
        }
        self.publish_report(shell, endpoints);
    }

    /// Answers a daemon's control message on the connection it arrived on.
    /// If that connection is gone, its daemon has already failed the jump or
    /// cancelled the picker.
    fn answer(
        &mut self,
        shell: &ClientShellState,
        endpoints: &mut EndpointRegistry,
        origin: &ControlOrigin,
        prefix: &str,
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
        let request = self.request(prefix, method);
        let Some(encoded) = encode(&request) else {
            return;
        };
        // A daemon closes a connection that sends a larger command than it
        // allows. The shell keeps a picker's answer within one command
        // (`pick_answer_fits`), and a jump's answer is small.
        if !fits(&encoded) {
            warn!(id = %request.id, "answer exceeds the endpoint command size; not sent");
            return;
        }
        send(endpoints, &origin.endpoint_id, boot_id, encoded);
    }

    /// Sends the report to the Local daemon when it differs from the one that
    /// daemon holds for this connection. A client without a Local connection,
    /// or whose Local daemon does not take reports, reports to nobody.
    fn publish_report(&mut self, shell: &ClientShellState, endpoints: &mut EndpointRegistry) {
        let local = ClientEndpointId::Local;
        let Some(connection) = endpoints.connection(&local) else {
            self.sent = None;
            return;
        };
        if !connection.negotiation.supports_method("client.report") {
            return;
        }
        let generation = connection.generation;
        // The request names the daemon's boot, which its first snapshot brings.
        let Some((boot_id, _)) = shell.endpoint_snapshot_identity(&local, generation) else {
            return;
        };
        let fingerprint = shell.client_report_fingerprint();
        let held = self
            .sent
            .as_ref()
            .filter(|sent| sent.generation == generation && sent.boot_id == boot_id);
        if held.is_some_and(|sent| sent.fingerprint == fingerprint) {
            return;
        }
        let report = match shell.client_report() {
            Ok(report) => report,
            Err(error) => {
                warn!(reason = error.message(), "client report could not be built");
                return;
            }
        };
        let changed = held.is_none_or(|sent| sent.report != report);
        let boot_id = boot_id.to_owned();
        if changed {
            if let Some(request) = self.encode_report(&report) {
                send(endpoints, &local, boot_id.clone(), request);
            }
        }
        // Kept even when the send failed: a failed transport is replaced by a
        // new connection, which reports afresh.
        self.sent = Some(SentReport {
            generation,
            boot_id,
            fingerprint,
            report,
        });
    }

    /// The report as one request the daemon will read. A daemon closes a
    /// connection that sends a larger command than it allows, so a report
    /// that is still too large after its text was cut loses rows from the
    /// end of the list until it fits.
    fn encode_report(&mut self, report: &ClientReport) -> Option<String> {
        let mut request = self.request(REPORT_REQUEST_PREFIX, Method::ClientReport(report.clone()));
        loop {
            let encoded = encode(&request)?;
            if fits(&encoded) {
                return Some(encoded);
            }
            let Method::ClientReport(report) = &mut request.method else {
                return None;
            };
            if report.agents.is_empty() {
                return None;
            }
            let keep = (report.agents.len() * MAX_ENDPOINT_COMMAND_BYTES / encoded.len())
                .min(report.agents.len() - 1);
            warn!(
                rows = report.agents.len(),
                keep, "client report exceeds the endpoint command size; dropping trailing rows"
            );
            report.agents.truncate(keep);
        }
    }

    fn request(&mut self, prefix: &str, method: Method) -> Request {
        self.next_request += 1;
        Request {
            id: format!("{prefix}{}", self.next_request),
            method,
        }
    }
}

/// Whether a rejection only says that this client's answer crossed the
/// daemon's own end of what it answers: a jump the daemon replaced with a
/// newer one, or a picker its plugin closed. The daemon had forgotten it by
/// the time the answer arrived, so `jump_not_found` or `pick_not_found` is
/// the expected reply and nothing is wrong.
fn answer_crossed_its_end(request_id: &str, code: Option<&str>) -> bool {
    [
        (JUMP_REQUEST_PREFIX, "jump_not_found"),
        (PICK_REQUEST_PREFIX, "pick_not_found"),
    ]
    .iter()
    .any(|(prefix, expected)| request_id.starts_with(prefix) && code == Some(*expected))
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
