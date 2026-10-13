//! `endpoint.client-jump.v1`: a daemon forwards a tool's `client.jump`, and
//! this client shows the pane, switching endpoint when it has to. The answer
//! goes back as `client.jump.resolve` on the connection the jump arrived on.
//!
//! A jump is answered by observation. The client navigates the way a click on
//! the agent does, and nothing follows that navigation's own request or
//! activation: the jump is `focused` once the client presents the endpoint
//! with the pane focused, and `timeout` when that has not happened in time.
//! `focused` is never sent in the client-loop turn that dispatched the
//! navigation, when the pane shown predates it, nor while the loop is still
//! switching endpoint, which the loop says each time it settles jumps.
//! This module decides where a jump leads and how it ended, from the retained
//! projection and explicit times, so it runs without a socket. It is mirrored
//! client logic: Herdr GPUI's counterpart is
//! `crates/herdr-gpui/src/client_contract/jump.rs` (see "Sibling fork: Herdr
//! GPUI" in `FORK.md`).
//!
//! The newest jump wins. A jump that arrives while another is pending
//! replaces it, and the replaced one is never answered: its daemon answered
//! its caller `jump_superseded` when it forwarded the newer one. The replaced
//! jump's navigation may still be under way, so the newer jump always
//! navigates, even to the pane already presented: its navigation is then the
//! last one dispatched, and the answer waits for any switch to finish, so it
//! is the one the client ends on.

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

impl ClientEndpointId {
    /// The endpoint a [`Self::storage_key`] names, as tools and daemons send
    /// it back.
    pub(crate) fn from_storage_key(key: &str) -> Option<Self> {
        if key == "local" {
            return Some(Self::Local);
        }
        crate::client::endpoint::ProfileId::parse(key.strip_prefix("ssh:")?)
            .ok()
            .map(Self::Ssh)
    }
}

#[derive(Debug)]
struct PendingClientJump {
    origin: ControlOrigin,
    jump: EndpointClientJump,
    deadline: Instant,
    /// Set until the end of the client-loop turn that dispatched the jump's
    /// navigation. The pane shown then predates that navigation.
    navigation_dispatched: bool,
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
    /// Whether the client presents `endpoint_id` with `pane_id` focused.
    fn presents_pane(&self, endpoint_id: &ClientEndpointId, pane_id: &str) -> bool {
        &self.active_endpoint_id == endpoint_id
            && self
                .snapshot
                .as_deref()
                .is_some_and(|snapshot| snapshot.focused_pane_id.as_deref() == Some(pane_id))
    }

    /// Starts a jump, in place of the pending one, if any. The returned
    /// input carries the navigation to dispatch; the answer comes from
    /// [`Self::settle_client_jumps`].
    pub(crate) fn receive_client_jump(
        &mut self,
        origin: ControlOrigin,
        jump: EndpointClientJump,
        now: Instant,
    ) -> ClientShellInput {
        let mut outcome = ClientShellInput::default();
        let replaced = self.fork.client_jumps.pending.take();
        let endpoint_id = match jump_target(&self.endpoints, &jump) {
            Ok(endpoint_id) => endpoint_id,
            Err(result) => {
                self.resolve_client_jump(origin, jump.jump_id, result);
                return outcome;
            }
        };
        // The replaced jump's navigation may still land elsewhere, so only a
        // client with no other jump skips the navigation to the pane it
        // presents. Settlement still answers it, once no switch is under way.
        let navigate = replaced.is_some() || !self.presents_pane(&endpoint_id, &jump.pane_id);
        if navigate {
            self.focus_or_activate(
                endpoint_id,
                ClientEndpointFocusTarget::Pane(jump.pane_id.clone()),
                &mut outcome,
            );
        }
        self.fork.client_jumps.pending = Some(PendingClientJump {
            origin,
            jump,
            deadline: now + NAVIGATION_SETTLE_DEADLINE,
            navigation_dispatched: navigate,
        });
        outcome
    }

    /// Settles the pending jump against what the client now presents and
    /// returns every answer that is ready. Called once per client-loop turn,
    /// after its event; `switching` says whether the loop is still switching
    /// endpoint (an activation pending or scheduled, or Local's activation
    /// deferred), during which the endpoint presented is not yet settled.
    pub(crate) fn settle_client_jumps(
        &mut self,
        now: Instant,
        switching: bool,
    ) -> Vec<ResolvedClientJump> {
        if let Some(mut pending) = self.fork.client_jumps.pending.take() {
            let dispatched_this_turn = std::mem::take(&mut pending.navigation_dispatched);
            let settled = !dispatched_this_turn && !switching;
            let outcome = match jump_target(&self.endpoints, &pending.jump) {
                Err(outcome) => Some(outcome),
                Ok(endpoint_id)
                    if settled && self.presents_pane(&endpoint_id, &pending.jump.pane_id) =>
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
                None => self.fork.client_jumps.pending = Some(pending),
            }
        }
        std::mem::take(&mut self.fork.client_jumps.resolved)
    }

    fn resolve_client_jump(
        &mut self,
        origin: ControlOrigin,
        jump_id: String,
        outcome: ClientJumpOutcome,
    ) {
        self.fork.client_jumps.resolved.push(ResolvedClientJump {
            origin,
            jump_id,
            outcome,
        });
    }
}
