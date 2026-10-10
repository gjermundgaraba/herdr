//! `client.jump`: a tool asks one client to show a pane. The daemon forwards
//! the request and waits; the client navigates, switching endpoint when it
//! has to, and answers with `client.jump.resolve` on the same connection.
//! The daemon never navigates itself.
//!
//! The newest jump wins: a jump for a client that is still performing one
//! replaces it, and the replaced jump's caller is answered `jump_superseded`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::HeadlessServer;
use crate::api;
use crate::api::schema::{
    ClientJumpOutcome, ClientJumpParams, ClientJumpResolveParams, Method, ResponseResult,
    MAX_CLIENT_REPORT_ID_BYTES, MAX_CLIENT_REPORT_TEXT_BYTES,
};
use crate::app::{encode_error, encode_success};
use crate::protocol::endpoint::{client_jump_message, EndpointClientJump};

/// How long the daemon waits for a client's answer. Longer than a client
/// should take to give up on its own, so this only ends a jump whose client
/// stopped answering.
pub(super) const CLIENT_JUMP_TIMEOUT: Duration = Duration::from_secs(20);

struct PendingJump {
    jump_id: String,
    request_id: String,
    endpoint_id: String,
    pane_id: String,
    respond_to: std::sync::mpsc::Sender<String>,
    deadline: Instant,
}

/// Open jumps by client id: each client's newest one.
#[derive(Default)]
pub(super) struct ClientJumps {
    pending: HashMap<u64, PendingJump>,
    next_id: u64,
}

impl ClientJumps {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_CLIENT_REPORT_ID_BYTES
}

impl HeadlessServer {
    /// Forwards `client.jump` to its client and keeps the caller waiting.
    pub(super) fn start_client_jump(&mut self, msg: api::ApiRequestMessage) {
        let request_id = msg.request.id;
        let Method::ClientJump(ClientJumpParams {
            client_id,
            endpoint_id,
            boot_id,
            pane_id,
        }) = msg.request.method
        else {
            return;
        };
        let reject = |code: &str, message: &str| {
            let _ = msg
                .respond_to
                .send(encode_error(request_id.clone(), code, message));
        };
        if ![&endpoint_id, &boot_id, &pane_id]
            .into_iter()
            .all(|id| valid_id(id))
        {
            reject(
                "invalid_params",
                "endpoint_id, boot_id, and pane_id must be 1 to 128 bytes",
            );
            return;
        }
        if !self
            .clients
            .get(&client_id)
            .is_some_and(|client| client.is_shell_client())
        {
            reject("client_not_found", "no client with that id is attached");
            return;
        }
        // A client that never reported does not answer jumps either.
        if !self.client_reports.contains(client_id) {
            reject(
                "client_not_reporting",
                "that client has not sent a report, so it cannot be jumped",
            );
            return;
        }
        self.client_jumps.next_id += 1;
        let jump_id = format!("jump-{}", self.client_jumps.next_id);
        let sent = client_jump_message(&EndpointClientJump {
            jump_id: jump_id.clone(),
            endpoint_id: endpoint_id.clone(),
            boot_id,
            pane_id: pane_id.clone(),
        })
        .is_ok_and(|message| self.send_to_client(client_id, message));
        if !sent {
            reject("client_departed", "the client went away");
            return;
        }
        // The client drops the jump it was performing for this one, so that
        // jump's caller hears now instead of waiting for an answer that will
        // not come.
        self.finish_client_jump(
            client_id,
            Err((
                "jump_superseded",
                "a newer jump to that client replaced this one",
            )),
        );
        self.client_jumps.pending.insert(
            client_id,
            PendingJump {
                jump_id,
                request_id,
                endpoint_id,
                pane_id,
                respond_to: msg.respond_to,
                deadline: Instant::now() + CLIENT_JUMP_TIMEOUT,
            },
        );
    }

    /// Applies `client.jump.resolve` from the client lane.
    pub(super) fn resolve_client_jump(
        &mut self,
        client_id: u64,
        params: ClientJumpResolveParams,
    ) -> Result<ResponseResult, (&'static str, String)> {
        let owned = self
            .client_jumps
            .pending
            .get(&client_id)
            .is_some_and(|jump| jump.jump_id == params.jump_id);
        if !owned {
            return Err((
                "jump_not_found",
                "no open jump with that id on this client".to_owned(),
            ));
        }
        self.finish_client_jump(client_id, Ok(params.outcome));
        Ok(ResponseResult::Ok {})
    }

    /// Fails the jump a departing client was performing.
    pub(super) fn cancel_client_jump(&mut self, client_id: u64) {
        self.finish_client_jump(
            client_id,
            Err((
                "client_departed",
                "the client detached before it answered the jump",
            )),
        );
    }

    /// Fails the jumps whose client never answered.
    pub(super) fn expire_client_jumps(&mut self, now: Instant) {
        let client_ids = self
            .client_jumps
            .pending
            .iter()
            .filter(|(_, jump)| now >= jump.deadline)
            .map(|(client_id, _)| *client_id)
            .collect::<Vec<_>>();
        for client_id in client_ids {
            self.finish_client_jump(
                client_id,
                Err(("jump_timeout", "the client did not answer the jump in time")),
            );
        }
    }

    /// Answers the waiting caller of the client's jump: a client outcome, or
    /// a daemon-side failure.
    fn finish_client_jump(
        &mut self,
        client_id: u64,
        outcome: Result<ClientJumpOutcome, (&'static str, &str)>,
    ) {
        let Some(jump) = self.client_jumps.pending.remove(&client_id) else {
            return;
        };
        let failure =
            |code: &str, message: &str| encode_error(jump.request_id.clone(), code, message);
        let response = match outcome {
            Ok(ClientJumpOutcome::Focused) => encode_success(
                jump.request_id,
                ResponseResult::ClientJumped {
                    client_id,
                    endpoint_id: jump.endpoint_id,
                    pane_id: jump.pane_id,
                },
            ),
            Ok(ClientJumpOutcome::StaleBoot) => failure(
                "stale_boot",
                "the endpoint restarted since that pane id was read",
            ),
            Ok(ClientJumpOutcome::NotFound) => failure(
                "jump_target_not_found",
                "the client knows no such endpoint or pane",
            ),
            Ok(ClientJumpOutcome::Unavailable) => failure(
                "jump_endpoint_unavailable",
                "the endpoint is not online, or the client's navigation did not reach it or was withdrawn",
            ),
            Ok(ClientJumpOutcome::Timeout) => failure(
                "jump_timeout",
                "the client did not see the pane focused in time",
            ),
            Ok(ClientJumpOutcome::Failed { message }) => {
                let message = bounded_message(&message);
                failure(
                    "jump_failed",
                    if message.is_empty() {
                        "the client could not perform the jump"
                    } else {
                        message
                    },
                )
            }
            Err((code, message)) => failure(code, message),
        };
        let _ = jump.respond_to.send(response);
    }
}

/// A client's failure text, cut to the report text limit.
fn bounded_message(message: &str) -> &str {
    let message = message.trim();
    &message[..message.floor_char_boundary(MAX_CLIENT_REPORT_TEXT_BYTES)]
}
