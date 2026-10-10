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

use std::time::{Duration, Instant};

use super::*;
use crate::api::schema::ClientJumpOutcome;
use crate::protocol::endpoint::EndpointClientJump;

/// How long a navigation may take to show its target as focused before the
/// client reports that it did not settle. The daemon stops waiting for a jump
/// after 20 seconds, so a client answers well before that.
pub(crate) const NAVIGATION_SETTLE_DEADLINE: Duration = Duration::from_secs(12);

/// The connection a jump arrived on, which its answer goes back on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClientJumpOrigin {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) generation: u64,
}

#[derive(Debug)]
struct PendingClientJump {
    origin: ClientJumpOrigin,
    jump: EndpointClientJump,
    endpoint_id: ClientEndpointId,
    deadline: Instant,
}

/// A jump whose answer is ready to send.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedClientJump {
    pub(crate) origin: ClientJumpOrigin,
    pub(crate) jump_id: String,
    pub(crate) outcome: ClientJumpOutcome,
}

/// Jumps this client is performing, and answers not yet collected.
#[derive(Debug, Default)]
pub(crate) struct ClientJumps {
    pending: Vec<PendingClientJump>,
    resolved: Vec<ResolvedClientJump>,
}

/// The endpoint a jump leads to, or why it cannot be performed. The pane id
/// is only meaningful under the boot it was read from, so a boot the retained
/// snapshot no longer has is checked before anything else about the endpoint.
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
    pub(crate) fn presents_target(
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

    /// Starts a jump. The returned input carries the navigation to dispatch;
    /// the answer comes from [`Self::settle_client_jumps`].
    pub(crate) fn receive_client_jump(
        &mut self,
        origin: ClientJumpOrigin,
        jump: EndpointClientJump,
        now: Instant,
    ) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        let endpoint_id = match jump_target(&self.endpoints, &jump) {
            Ok(endpoint_id) => endpoint_id,
            Err(result) => {
                self.resolve_client_jump(origin, jump.jump_id, result);
                return outcome;
            }
        };
        let target = ClientEndpointFocusTarget::Pane(jump.pane_id.clone());
        if self.presents_target(&endpoint_id, &target) {
            self.resolve_client_jump(origin, jump.jump_id, ClientJumpOutcome::Focused);
            return outcome;
        }
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
        self.client_jumps.pending.push(PendingClientJump {
            origin,
            jump,
            endpoint_id,
            deadline: now + NAVIGATION_SETTLE_DEADLINE,
        });
        outcome
    }

    /// Ends the jump whose focus request its endpoint rejected or never
    /// answered.
    pub(super) fn fail_client_jump(
        &mut self,
        origin: &ClientJumpOrigin,
        jump_id: &str,
        error: &ClientShellEndpointError,
    ) {
        let Some(index) = self
            .client_jumps
            .pending
            .iter()
            .position(|pending| &pending.origin == origin && pending.jump.jump_id == jump_id)
        else {
            return;
        };
        let pending = self.client_jumps.pending.remove(index);
        let outcome = match error.code.as_deref() {
            Some("pane_not_found") => ClientJumpOutcome::NotFound,
            Some("stale_boot") => ClientJumpOutcome::StaleBoot,
            Some("endpoint_timeout") => ClientJumpOutcome::Timeout,
            _ => ClientJumpOutcome::Failed {
                message: error.message.clone(),
            },
        };
        self.resolve_client_jump(pending.origin, pending.jump.jump_id, outcome);
    }

    /// Settles the open jumps against what the client now presents and
    /// returns every answer that is ready. Called once per client-loop turn.
    pub(crate) fn settle_client_jumps(&mut self, now: Instant) -> Vec<ResolvedClientJump> {
        for pending in std::mem::take(&mut self.client_jumps.pending) {
            let target = ClientEndpointFocusTarget::Pane(pending.jump.pane_id.clone());
            let outcome = match jump_target(&self.endpoints, &pending.jump) {
                Err(outcome) => Some(outcome),
                Ok(_) if self.presents_target(&pending.endpoint_id, &target) => {
                    Some(ClientJumpOutcome::Focused)
                }
                Ok(_) if now >= pending.deadline => Some(ClientJumpOutcome::Timeout),
                Ok(_) => None,
            };
            match outcome {
                Some(outcome) => {
                    self.resolve_client_jump(pending.origin, pending.jump.jump_id, outcome)
                }
                None => self.client_jumps.pending.push(pending),
            }
        }
        std::mem::take(&mut self.client_jumps.resolved)
    }

    fn resolve_client_jump(
        &mut self,
        origin: ClientJumpOrigin,
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
