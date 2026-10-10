use super::*;

#[derive(Clone, Debug)]
pub(crate) struct ClientEndpointAgentViewProjection {
    generation: Option<u64>,
    boot_id: String,
    revision: u64,
    pub(crate) view: Result<Option<crate::api::schema::AgentViewSetParams>, ()>,
}

#[derive(Clone, Debug)]
pub(crate) struct ClientShellEndpoint {
    pub(crate) endpoint_id: ClientEndpointId,
    pub(crate) label: String,
    pub(crate) status: ClientEndpointStatus,
    pub(crate) snapshot: Option<Box<ClientShellSnapshot>>,
    /// Connection generation that produced `snapshot`. `None` is reserved for local tests.
    pub(crate) snapshot_generation: Option<u64>,
    pub(crate) agent_view_projection: Option<ClientEndpointAgentViewProjection>,
    pending_agent_view_projection: Option<ClientEndpointAgentViewProjection>,
    pub(crate) agent_view_projection_supported: bool,
    pub(crate) methods: Option<HashSet<String>>,
}

pub(super) struct MachineHit {
    pub(super) rect: Rect,
    pub(super) status_badge: Rect,
    pub(super) collapse_toggle: Rect,
    pub(super) endpoint_id: ClientEndpointId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClientEndpointFocusTarget {
    Workspace(String),
    Tab(String),
    Pane(String),
    #[cfg(windows)]
    Notification {
        pane_id: String,
        boot_id: String,
    },
}

impl ClientShellEndpoint {
    /// Which agent view projection the endpoint holds: at most one exists per
    /// connection, boot, and snapshot revision.
    pub(super) fn agent_view_identity(&self) -> Option<(Option<u64>, &str, u64)> {
        self.agent_view_projection.as_ref().map(|projection| {
            (
                projection.generation,
                projection.boot_id.as_str(),
                projection.revision,
            )
        })
    }
}

impl ClientShellState {
    pub(crate) fn set_endpoint_catalog(&mut self, profiles: &[SavedSshEndpoint]) {
        let mut next = Vec::with_capacity(profiles.len().saturating_add(1));
        let local = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id.is_local())
            .cloned()
            .unwrap_or_else(local_endpoint);
        next.push(local);
        for profile in profiles {
            let endpoint_id = ClientEndpointId::Ssh(profile.id.clone());
            let previous = self
                .endpoints
                .iter()
                .find(|endpoint| endpoint.endpoint_id == endpoint_id)
                .filter(|endpoint| {
                    profile.enabled && endpoint.status != ClientEndpointStatus::Disabled
                });
            next.push(ClientShellEndpoint {
                endpoint_id,
                label: profile.label.clone(),
                status: previous.map_or(
                    if profile.enabled {
                        ClientEndpointStatus::Connecting
                    } else {
                        ClientEndpointStatus::Disabled
                    },
                    |endpoint| endpoint.status,
                ),
                snapshot: previous.and_then(|endpoint| endpoint.snapshot.clone()),
                snapshot_generation: previous.and_then(|endpoint| endpoint.snapshot_generation),
                agent_view_projection: previous
                    .and_then(|endpoint| endpoint.agent_view_projection.clone()),
                pending_agent_view_projection: previous
                    .and_then(|endpoint| endpoint.pending_agent_view_projection.clone()),
                agent_view_projection_supported: previous
                    .is_some_and(|endpoint| endpoint.agent_view_projection_supported),
                methods: previous.and_then(|endpoint| endpoint.methods.clone()),
            });
        }

        if !next
            .iter()
            .any(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
        {
            self.select_unavailable_local();
        }
        self.collapsed_endpoints.retain(|endpoint_id| {
            next.iter()
                .any(|endpoint| &endpoint.endpoint_id == endpoint_id)
        });
        for endpoint in &self.endpoints {
            if !next.iter().any(|candidate| {
                candidate.endpoint_id == endpoint.endpoint_id
                    && candidate.status != ClientEndpointStatus::Disabled
            }) {
                self.history.retire(&endpoint.endpoint_id);
            }
        }
        self.endpoints = next;
    }

    pub(crate) fn select_unavailable_local(&mut self) {
        self.history.clear();
        self.reset_endpoint_projection();
        self.active_endpoint_id = ClientEndpointId::Local;
        self.mode = ClientShellMode::Terminal;
        self.snapshot = None;
        self.graphics.set_scope("local:unavailable");
        self.reconcile_input_source();
    }

    pub(crate) fn retire_endpoint(&mut self, endpoint_id: &ClientEndpointId) {
        self.clear_machine_diagnostic(endpoint_id);
        if endpoint_id == &self.active_endpoint_id {
            self.pending_workspace_highlight = None;
        }
        self.history.retire(endpoint_id);
        self.retire_endpoint_notifications(endpoint_id);
        if let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        {
            endpoint.status = ClientEndpointStatus::Disabled;
            endpoint.snapshot = None;
            endpoint.snapshot_generation = None;
            endpoint.methods = None;
            endpoint.agent_view_projection = None;
            endpoint.pending_agent_view_projection = None;
            endpoint.agent_view_projection_supported = false;
        }
    }

    pub(crate) fn set_endpoint_status(
        &mut self,
        endpoint_id: &ClientEndpointId,
        status: ClientEndpointStatus,
    ) {
        if matches!(
            status,
            ClientEndpointStatus::Online | ClientEndpointStatus::Disabled
        ) {
            self.clear_machine_diagnostic(endpoint_id);
        }
        if endpoint_id == &self.active_endpoint_id && status != ClientEndpointStatus::Online {
            self.pending_workspace_highlight = None;
        }
        if let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        {
            endpoint.status = status;
        }
    }

    pub(crate) fn mark_endpoint_disconnected(&mut self, endpoint_id: &ClientEndpointId) {
        self.set_endpoint_status(endpoint_id, ClientEndpointStatus::Reconnecting);
        if endpoint_id == &self.active_endpoint_id {
            let pending = self.pending_requests.keys().cloned().collect::<Vec<_>>();
            for request_id in pending {
                self.cancel_endpoint_request(&request_id);
            }
            self.pending_integration_installs = 0;
            self.pane_scroll_in_flight.clear();
            self.pane_scroll_queued.clear();
            // The picker goes with the connection that opened it, whose
            // daemon cancels it when the connection drops. The answer names
            // that connection, so it is never sent on a later one.
            self.cancel_pick();
        }
    }

    pub(crate) fn set_endpoint_agent_view_projection_supported(
        &mut self,
        endpoint_id: &ClientEndpointId,
        supported: bool,
    ) {
        if let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        {
            endpoint.agent_view_projection_supported = supported;
            if !supported {
                endpoint.agent_view_projection = None;
                endpoint.pending_agent_view_projection = None;
            }
        }
    }

    pub(crate) fn set_endpoint_methods_for(
        &mut self,
        endpoint_id: &ClientEndpointId,
        methods: Option<Vec<String>>,
    ) {
        let methods = methods.map(|methods| methods.into_iter().collect::<HashSet<_>>());
        if let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        {
            endpoint.methods = methods;
        }
    }

    pub(crate) fn endpoint_projection_available(&self, endpoint_id: &ClientEndpointId) -> bool {
        self.endpoints.iter().any(|endpoint| {
            &endpoint.endpoint_id == endpoint_id
                && endpoint.status == ClientEndpointStatus::Online
                && endpoint.snapshot.is_some()
        })
    }

    pub(crate) fn activate_endpoint_projection(&mut self, endpoint_id: &ClientEndpointId) -> bool {
        let pending_agent_reveal = self
            .pending_agent_reveal
            .take_if(|(target_endpoint, _)| target_endpoint == endpoint_id);
        let agent_body_height = self.hits.agent_body.height;
        let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        else {
            return false;
        };
        if endpoint.status != ClientEndpointStatus::Online {
            return false;
        }
        let Some(snapshot) = endpoint.snapshot.clone() else {
            return false;
        };
        let generation = endpoint.snapshot_generation;
        let switching_endpoint = endpoint_id != &self.active_endpoint_id;
        let agent_scroll = self.agent_scroll;
        if switching_endpoint {
            self.active_endpoint_id = endpoint_id.clone();
            self.pane_surface = None;
            self.pending_pane_surface = None;
            // A picker belongs to the endpoint that opened it.
            self.cancel_pick();
        }
        self.apply_active_snapshot(snapshot, generation);
        if switching_endpoint {
            // The aggregate agent list belongs to the client, not one endpoint.
            self.agent_scroll = agent_scroll;
        }
        if let Some((_, pane_id)) = pending_agent_reveal {
            self.reveal_endpoint_agent(endpoint_id, &pane_id, agent_body_height);
        }
        true
    }

    pub(crate) fn endpoint_status(
        &self,
        endpoint_id: &ClientEndpointId,
    ) -> Option<ClientEndpointStatus> {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .map(|endpoint| endpoint.status)
    }

    pub(crate) fn endpoint_has_snapshot(&self, endpoint_id: &ClientEndpointId) -> bool {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .is_some_and(|endpoint| endpoint.snapshot.is_some())
    }

    pub(crate) fn endpoint_is_online(&self, endpoint_id: &ClientEndpointId) -> bool {
        self.endpoint_status(endpoint_id) == Some(ClientEndpointStatus::Online)
            && self.endpoint_has_snapshot(endpoint_id)
    }

    pub(crate) fn endpoint_boot_id(&self, endpoint_id: &ClientEndpointId) -> Option<&str> {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)?
            .snapshot
            .as_deref()
            .map(|snapshot| snapshot.boot_id.as_str())
    }

    pub(crate) fn endpoint_snapshot_matches(
        &self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        boot_id: &str,
        revision: u64,
    ) -> bool {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .is_some_and(|endpoint| {
                endpoint
                    .snapshot_generation
                    .is_none_or(|snapshot_generation| snapshot_generation == generation)
                    && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                        snapshot.boot_id == boot_id && snapshot.revision == revision
                    })
            })
    }

    pub(crate) fn endpoint_snapshot_identity(
        &self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
    ) -> Option<(&str, u64)> {
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)?;
        if endpoint
            .snapshot_generation
            .is_some_and(|snapshot_generation| snapshot_generation != generation)
        {
            return None;
        }
        endpoint
            .snapshot
            .as_deref()
            .map(|snapshot| (snapshot.boot_id.as_str(), snapshot.revision))
    }

    pub(crate) fn set_endpoint_agent_view_projection_for_generation(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        projection: crate::client::endpoint::DecodedAgentViewProjection,
    ) {
        self.set_endpoint_agent_view_projection(
            endpoint_id,
            Some(generation),
            projection.boot_id,
            projection.revision,
            projection.view,
        );
    }

    #[cfg(test)]
    pub(crate) fn set_test_endpoint_agent_view(
        &mut self,
        endpoint_id: &ClientEndpointId,
        view: Option<crate::api::schema::AgentViewSetParams>,
    ) {
        let Some((boot_id, revision)) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| {
                endpoint
                    .snapshot
                    .as_deref()
                    .map(|snapshot| (snapshot.boot_id.clone(), snapshot.revision))
            })
        else {
            return;
        };
        self.set_endpoint_agent_view_projection_supported(endpoint_id, true);
        self.set_endpoint_agent_view_projection(endpoint_id, None, boot_id, revision, Ok(view));
    }

    #[cfg(test)]
    pub(crate) fn set_test_endpoint_agent_view_projection(
        &mut self,
        endpoint_id: &ClientEndpointId,
        boot_id: &str,
        revision: u64,
        view: Option<crate::api::schema::AgentViewSetParams>,
    ) {
        self.set_endpoint_agent_view_projection(
            endpoint_id,
            None,
            boot_id.to_owned(),
            revision,
            Ok(view),
        );
    }

    fn set_endpoint_agent_view_projection(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: Option<u64>,
        boot_id: String,
        revision: u64,
        view: Result<Option<crate::api::schema::AgentViewSetParams>, ()>,
    ) {
        let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        else {
            return;
        };
        if endpoint.snapshot_generation == generation
            && endpoint
                .snapshot
                .as_deref()
                .is_some_and(|snapshot| snapshot.boot_id == boot_id && snapshot.revision > revision)
        {
            return;
        }
        let next = ClientEndpointAgentViewProjection {
            generation,
            boot_id,
            revision,
            view,
        };
        let matches_snapshot = endpoint.snapshot_generation == next.generation
            && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot.boot_id == next.boot_id && snapshot.revision == next.revision
            });
        let slot = if matches_snapshot {
            &mut endpoint.agent_view_projection
        } else {
            &mut endpoint.pending_agent_view_projection
        };
        if slot.as_ref().is_some_and(|current| {
            current.generation == next.generation
                && current.boot_id == next.boot_id
                && current.revision >= next.revision
        }) {
            return;
        }
        *slot = Some(next);
    }

    pub(crate) fn endpoint_agent_view(
        endpoint: &ClientShellEndpoint,
    ) -> Option<&Result<Option<crate::api::schema::AgentViewSetParams>, ()>> {
        let snapshot = endpoint.snapshot.as_deref()?;
        let projection = endpoint.agent_view_projection.as_ref()?;
        (projection.generation == endpoint.snapshot_generation
            && projection.boot_id == snapshot.boot_id
            && projection.revision == snapshot.revision)
            .then_some(&projection.view)
    }

    /// A terminal normally starts focused. `None` means this host cannot report focus events,
    /// not that the endpoint has no viewer; activation therefore sends an explicit true baseline.
    pub(crate) fn host_focus_baseline(&self) -> bool {
        self.outer_focused.unwrap_or(true)
    }

    pub(crate) fn endpoint_label(&self, endpoint_id: &ClientEndpointId) -> &str {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .map_or("Unknown endpoint", |endpoint| endpoint.label.as_str())
    }

    pub(crate) fn active_endpoint_label(&self) -> &str {
        self.endpoint_label(&self.active_endpoint_id)
    }

    pub(crate) fn endpoint_is_active(&self, endpoint_id: &ClientEndpointId) -> bool {
        &self.active_endpoint_id == endpoint_id
    }

    pub(crate) fn multi_endpoint_active(&self) -> bool {
        self.endpoints.len() > 1
    }

    #[cfg(test)]
    pub(crate) fn set_snapshot(&mut self, snapshot: Box<ClientShellSnapshot>) {
        let endpoint_id = self.active_endpoint_id.clone();
        self.set_endpoint_snapshot(&endpoint_id, snapshot);
    }

    #[cfg(test)]
    pub(crate) fn set_endpoint_methods(&mut self, methods: Option<Vec<String>>) {
        let endpoint_id = self.active_endpoint_id.clone();
        self.set_endpoint_methods_for(&endpoint_id, methods);
    }

    pub(super) fn supports_endpoint_method(&self, method: &crate::api::schema::Method) -> bool {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .and_then(|endpoint| endpoint.methods.as_ref())
            .is_none_or(|methods| methods.contains(crate::api::api_method_name(method)))
    }

    pub(super) fn focused_tab_count(&self) -> usize {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return 0;
        };
        snapshot
            .tabs
            .iter()
            .filter(|tab| {
                Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref()
            })
            .count()
    }

    #[cfg(test)]
    pub(crate) fn cache_endpoint_snapshot(
        &mut self,
        endpoint_id: &ClientEndpointId,
        snapshot: Box<ClientShellSnapshot>,
    ) {
        self.store_endpoint_snapshot(endpoint_id, None, snapshot);
    }

    pub(crate) fn cache_endpoint_snapshot_for_generation(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        snapshot: Box<ClientShellSnapshot>,
    ) {
        self.store_endpoint_snapshot(endpoint_id, Some(generation), snapshot);
    }

    /// Retains an endpoint's snapshot as its daemon sent it. Agent statuses,
    /// and so completion badges, are the daemon's: this client keeps no
    /// record of what it has shown.
    fn store_endpoint_snapshot(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: Option<u64>,
        mut snapshot: Box<ClientShellSnapshot>,
    ) {
        snapshot
            .commands
            .retain(|command| command.action != crate::protocol::ClientShellCommandAction::Unknown);
        let Some(index) = self
            .endpoints
            .iter()
            .position(|endpoint| &endpoint.endpoint_id == endpoint_id)
        else {
            return;
        };
        if self.endpoints[index].snapshot_generation == generation
            && self.endpoints[index]
                .snapshot
                .as_deref()
                .is_some_and(|previous| {
                    previous.boot_id == snapshot.boot_id && previous.revision > snapshot.revision
                })
        {
            return;
        }
        // A server reboot invalidates history visits recorded under its old boot,
        // and closed panes invalidate theirs, even while the endpoint is inactive.
        self.history.observe_boot(endpoint_id, &snapshot.boot_id);
        self.history.observe_panes(
            endpoint_id,
            &snapshot.boot_id,
            snapshot
                .panes
                .iter()
                .map(|pane| pane.pane_id.as_str())
                .chain(snapshot.focused_pane_id.as_deref()),
        );
        let boot_changed = self.endpoints[index]
            .snapshot
            .as_deref()
            .is_some_and(|previous| previous.boot_id != snapshot.boot_id);
        if boot_changed {
            self.retire_endpoint_notifications(endpoint_id);
        }
        let endpoint = &mut self.endpoints[index];
        endpoint.snapshot_generation = generation;
        endpoint.snapshot = Some(snapshot);
        let pending_matches =
            endpoint
                .pending_agent_view_projection
                .as_ref()
                .is_some_and(|projection| {
                    projection.generation == generation
                        && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                            projection.boot_id == snapshot.boot_id
                                && projection.revision == snapshot.revision
                        })
                });
        if pending_matches {
            endpoint.agent_view_projection = endpoint.pending_agent_view_projection.take();
        } else {
            endpoint.pending_agent_view_projection = None;
            if endpoint
                .agent_view_projection
                .as_ref()
                .is_some_and(|projection| {
                    projection.generation != generation
                        || endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                            projection.boot_id != snapshot.boot_id
                                || projection.revision != snapshot.revision
                        })
                })
            {
                endpoint.agent_view_projection = None;
            }
        }
    }

    /// Asks the daemon that owns the focused pane to hold its agent's
    /// acknowledged completion (`agent.mark_unread`). The daemon keeps the
    /// hold and decides when the badge comes back and when it clears, so this
    /// client records nothing: it shows what the next snapshots say. Herdr
    /// GPUI's counterpart is `crates/herdr-gpui/src/window/commands.rs`, with
    /// the daemon's answer handled in `client_contract/follow.rs`.
    pub(super) fn mark_focused_pane_unread(&mut self, outcome: &mut ClientShellInput) {
        let Some(pane_id) = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone())
        else {
            outcome.repaint |= self.push_endpoint_notice(
                ClientEndpointNoticeKind::Rejected,
                "mark_unread",
                "Nothing to mark",
                "No pane is focused.",
            );
            return;
        };
        self.push_endpoint_method_with_kind(
            crate::api::schema::Method::AgentMarkUnread(crate::api::schema::PaneTarget { pane_id }),
            PendingEndpointKind::MarkUnread,
            outcome,
        );
    }

    #[cfg(test)]
    pub(crate) fn set_endpoint_snapshot(
        &mut self,
        endpoint_id: &ClientEndpointId,
        snapshot: Box<ClientShellSnapshot>,
    ) {
        self.cache_endpoint_snapshot(endpoint_id, snapshot);
        self.apply_cached_endpoint_snapshot(endpoint_id);
    }

    pub(crate) fn set_endpoint_snapshot_for_generation(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        snapshot: Box<ClientShellSnapshot>,
    ) {
        self.cache_endpoint_snapshot_for_generation(endpoint_id, generation, snapshot);
        self.apply_cached_endpoint_snapshot(endpoint_id);
    }

    fn apply_cached_endpoint_snapshot(&mut self, endpoint_id: &ClientEndpointId) {
        let Some((snapshot, generation)) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .and_then(|endpoint| {
                endpoint
                    .snapshot
                    .clone()
                    .map(|snapshot| (snapshot, endpoint.snapshot_generation))
            })
        else {
            return;
        };
        if endpoint_id == &self.active_endpoint_id {
            self.apply_active_snapshot(snapshot, generation);
        }
    }
}

pub(super) fn endpoint_status_presentation(
    status: ClientEndpointStatus,
    palette: &Palette,
) -> (&'static str, &'static str, ratatui::style::Color) {
    match status {
        ClientEndpointStatus::Connecting => ("◐", "connecting", palette.yellow),
        ClientEndpointStatus::Online => ("●", "online", palette.green),
        ClientEndpointStatus::Reconnecting => ("◐", "reconnecting", palette.yellow),
        ClientEndpointStatus::Attention => ("!", "attention", palette.red),
        ClientEndpointStatus::Disabled => ("·", "disabled", palette.overlay0),
    }
}

pub(super) fn local_endpoint() -> ClientShellEndpoint {
    ClientShellEndpoint {
        endpoint_id: ClientEndpointId::Local,
        label: "Local".into(),
        status: ClientEndpointStatus::Online,
        snapshot: None,
        snapshot_generation: None,
        agent_view_projection: None,
        pending_agent_view_projection: None,
        agent_view_projection_supported: false,
        methods: None,
    }
}
