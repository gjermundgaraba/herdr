//! `endpoint.client-jump.v1`: a daemon forwards a tool's `client.jump`, and
//! this client shows the pane, switching endpoint when it has to. The answer
//! goes back as `client.jump.resolve` on the connection the jump arrived on.
//!
//! The navigation is the client's own: the same focus request or endpoint
//! activation a click on an agent row starts. This module only decides where a
//! jump leads and how it ended, from the retained projection and explicit
//! times, so it runs without a socket. It is mirrored client logic: Herdr GPUI
//! answers jumps by the same rules (see "Sibling fork: Herdr GPUI" in
//! `FORK.md`).
//!
//! The newest jump wins. A jump that arrives while another is still settling
//! replaces it, and the replaced one is never answered: its daemon answered
//! its caller `jump_superseded` when it forwarded the newer one. The replaced
//! jump's navigation may still be under way, so the newer jump always
//! navigates, even to the pane already presented: its navigation is then the
//! last one dispatched and the one the client ends on.
//!
//! A jump is answered `focused` only while the client loop holds no endpoint
//! activation. A handoff shows its target before it completes, and until it
//! does the loop takes no request for that endpoint and may still reverse
//! the handoff. A tool that jumps again on the answer therefore finds a
//! client that can carry the next jump out.

use std::time::{Duration, Instant};

use super::*;
use crate::api::schema::ClientJumpOutcome;
use crate::protocol::endpoint::EndpointClientJump;

/// How long a navigation may take to show its target as focused before the
/// client reports that it did not settle. The daemon stops waiting for a jump
/// after 20 seconds, so a client answers well before that.
pub(crate) const NAVIGATION_SETTLE_DEADLINE: Duration = Duration::from_secs(12);

/// The connection a daemon's control message, a jump or a picker, arrived
/// on. Its answer goes back on that connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ControlOrigin {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) generation: u64,
}

#[derive(Debug)]
struct PendingClientJump {
    origin: ControlOrigin,
    jump: EndpointClientJump,
    endpoint_id: ClientEndpointId,
    deadline: Instant,
}

/// A jump whose answer is ready to send.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedClientJump {
    pub(crate) origin: ControlOrigin,
    pub(crate) jump_id: String,
    pub(crate) outcome: ClientJumpOutcome,
}

/// The jump this client is performing, and answers not yet collected.
#[derive(Debug, Default)]
pub(crate) struct ClientJumps {
    pending: Option<PendingClientJump>,
    resolved: Vec<ResolvedClientJump>,
}

/// The endpoint a jump leads to, or why it cannot be performed. The checks
/// run in this order: the endpoint is known, it has sent a snapshot, the
/// snapshot is of the jump's boot, the endpoint is online, the snapshot has
/// the pane. The pane id is only meaningful under the boot it was read from,
/// so another boot is reported before the endpoint's status or its panes.
pub(super) fn jump_target(
    endpoints: &[ClientShellEndpoint],
    jump: &EndpointClientJump,
) -> Result<ClientEndpointId, ClientJumpOutcome> {
    let endpoint_id =
        ClientEndpointId::from_storage_key(&jump.endpoint_id).ok_or(ClientJumpOutcome::NotFound)?;
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == endpoint_id)
        .ok_or(ClientJumpOutcome::NotFound)?;
    let Some(snapshot) = endpoint.snapshot.as_deref() else {
        return Err(ClientJumpOutcome::Unavailable);
    };
    if snapshot.boot_id != jump.boot_id {
        return Err(ClientJumpOutcome::StaleBoot);
    }
    if endpoint.status != ClientEndpointStatus::Online {
        return Err(ClientJumpOutcome::Unavailable);
    }
    if !snapshot
        .panes
        .iter()
        .any(|pane| pane.pane_id == jump.pane_id)
    {
        return Err(ClientJumpOutcome::NotFound);
    }
    Ok(endpoint_id)
}

impl ClientShellState {
    /// Whether the client presents `endpoint_id` with `target` focused.
    fn presents_target(
        &self,
        endpoint_id: &ClientEndpointId,
        target: &ClientEndpointFocusTarget,
    ) -> bool {
        if &self.active_endpoint_id != endpoint_id {
            return false;
        }
        let Some(snapshot) = self.snapshot.as_deref() else {
            return false;
        };
        match target {
            ClientEndpointFocusTarget::Workspace(id) => {
                snapshot.focused_workspace_id.as_ref() == Some(id)
            }
            ClientEndpointFocusTarget::Tab(id) => snapshot.focused_tab_id.as_ref() == Some(id),
            ClientEndpointFocusTarget::Pane(id) => snapshot.focused_pane_id.as_ref() == Some(id),
        }
    }

    /// Starts a jump, in place of the one still settling, if any. The
    /// returned input carries the navigation to dispatch; the answer comes
    /// from [`Self::settle_client_jumps`].
    pub(crate) fn receive_client_jump(
        &mut self,
        origin: ControlOrigin,
        jump: EndpointClientJump,
        now: Instant,
    ) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        // The focus request of the replaced jump keeps its kind, whether the
        // shell sent it or the client loop routed it back, so a late
        // rejection of it raises no notice and answers nothing.
        let replaced = self.client_jumps.pending.take();
        let endpoint_id = match jump_target(&self.endpoints, &jump) {
            Ok(endpoint_id) => endpoint_id,
            Err(result) => {
                self.resolve_client_jump(origin, jump.jump_id, result);
                return outcome;
            }
        };
        let target = ClientEndpointFocusTarget::Pane(jump.pane_id.clone());
        // Only a client with no jump and no endpoint activation under way
        // can take what it presents as settled. The replaced jump's
        // navigation may still land, and an activation may still leave the
        // pane, so this one navigates after them.
        if replaced.is_none()
            && !self.endpoint_activation_held
            && self.presents_target(&endpoint_id, &target)
        {
            self.resolve_client_jump(origin, jump.jump_id, ClientJumpOutcome::Focused);
            return outcome;
        }
        // The kind stays with a focus request on the presented endpoint. An
        // activation sends no request the shell tracks, unless the loop
        // routes it back as one: see `routed_client_jump`.
        let navigating = self.focus_or_activate_with_kind(
            endpoint_id.clone(),
            target,
            PendingEndpointKind::ClientJump {
                origin: origin.clone(),
                jump_id: jump.jump_id.clone(),
            },
            &mut outcome,
        );
        // No request and no activation means the endpoint took nothing.
        if !navigating || outcome.actions.is_empty() {
            self.resolve_client_jump(origin, jump.jump_id, ClientJumpOutcome::Unavailable);
            return outcome;
        }
        self.client_jumps.pending = Some(PendingClientJump {
            origin,
            jump,
            endpoint_id,
            deadline: now + NAVIGATION_SETTLE_DEADLINE,
        });
        outcome
    }

    /// The open jump, as `(origin, jump id)`, when a focus on `pane_id` that
    /// the client loop routed back to the presented endpoint is its
    /// navigation. A jump to a pane on the endpoint already presented can
    /// leave the shell as an activation (see `focus_or_activate_with_kind`),
    /// and the loop, with no handoff to make of it, sends a focus request
    /// instead. That request takes the jump's kind, as on the direct route.
    pub(super) fn routed_client_jump(&self, pane_id: &str) -> Option<(ControlOrigin, String)> {
        self.client_jumps
            .pending
            .as_ref()
            .filter(|pending| {
                pending.endpoint_id == self.active_endpoint_id && pending.jump.pane_id == pane_id
            })
            .map(|pending| (pending.origin.clone(), pending.jump.jump_id.clone()))
    }

    /// Ends the jump whose routed focus request could not be sent to the
    /// presented endpoint, which therefore took nothing.
    pub(super) fn client_jump_not_sent(&mut self, origin: &ControlOrigin, jump_id: &str) {
        if let Some(pending) = self
            .client_jumps
            .pending
            .take_if(|pending| &pending.origin == origin && pending.jump.jump_id == jump_id)
        {
            self.resolve_client_jump(
                pending.origin,
                pending.jump.jump_id,
                ClientJumpOutcome::Unavailable,
            );
        }
    }

    /// Ends the jump whose focus request its endpoint rejected, or the
    /// client loop cancelled before the endpoint answered it. A request the
    /// endpoint leaves unanswered ends the jump at its deadline instead: the
    /// command lane gives up on a request much later than that.
    pub(super) fn fail_client_jump(
        &mut self,
        origin: &ControlOrigin,
        jump_id: &str,
        error: &ClientShellEndpointError,
    ) {
        // The request of a jump that was replaced meanwhile answers nothing.
        let Some(pending) = self
            .client_jumps
            .pending
            .take_if(|pending| &pending.origin == origin && pending.jump.jump_id == jump_id)
        else {
            return;
        };
        let outcome = match error.code.as_deref() {
            Some("pane_not_found") => ClientJumpOutcome::NotFound,
            Some("stale_boot") => ClientJumpOutcome::StaleBoot,
            // The client's own code for a request it withdrew: the loop
            // could not hand it to the connection, the endpoint
            // disconnected with it in flight, or the user activated another
            // machine. The endpoint rejected nothing.
            Some("endpoint_cancelled") => ClientJumpOutcome::Unavailable,
            _ => ClientJumpOutcome::Failed {
                message: error.message.clone(),
            },
        };
        self.resolve_client_jump(pending.origin, pending.jump.jump_id, outcome);
    }

    /// Settles the open jump against what the client now presents and
    /// returns every answer that is ready. Called once per client-loop turn,
    /// after the loop has said whether it holds an endpoint activation:
    /// while it does, the pane may be presented and the jump is still not
    /// over.
    pub(crate) fn settle_client_jumps(&mut self, now: Instant) -> Vec<ResolvedClientJump> {
        if let Some(pending) = self.client_jumps.pending.take() {
            let target = ClientEndpointFocusTarget::Pane(pending.jump.pane_id.clone());
            let outcome = match jump_target(&self.endpoints, &pending.jump) {
                Err(outcome) => Some(outcome),
                Ok(_)
                    if !self.endpoint_activation_held
                        && self.presents_target(&pending.endpoint_id, &target) =>
                {
                    Some(ClientJumpOutcome::Focused)
                }
                Ok(_) if now >= pending.deadline => Some(ClientJumpOutcome::Timeout),
                Ok(_) => None,
            };
            match outcome {
                Some(outcome) => {
                    self.resolve_client_jump(pending.origin, pending.jump.jump_id, outcome)
                }
                None => self.client_jumps.pending = Some(pending),
            }
        }
        std::mem::take(&mut self.client_jumps.resolved)
    }

    fn resolve_client_jump(
        &mut self,
        origin: ControlOrigin,
        jump_id: String,
        outcome: ClientJumpOutcome,
    ) {
        self.client_jumps.resolved.push(ResolvedClientJump {
            origin,
            jump_id,
            outcome,
        });
    }
}
