//! Frontend-socket observations derived from the same retained projection used
//! for rendering and input. The wire identifies an endpoint by the server boot
//! id of its retained snapshot; a stale capture fails one comparison.
use super::*;
use serde_json::{json, Value};

pub(crate) struct FrontendEndpointRoute<'a> {
    pub(crate) generation: u64,
    pub(crate) boot_id: Option<&'a str>,
    pub(crate) status: ClientEndpointStatus,
}

impl ClientShellEndpoint {
    fn frontend_route(
        &self,
        registry: &crate::client::endpoint::EndpointRegistry,
    ) -> FrontendEndpointRoute<'_> {
        FrontendEndpointRoute {
            generation: registry
                .connection(&self.endpoint_id)
                .map(|connection| connection.generation)
                .or(self.snapshot_generation)
                .unwrap_or(0),
            boot_id: self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.boot_id.as_str()),
            status: self.status,
        }
    }
}

impl ClientShellState {
    /// The retained snapshot's boot id a captured route must match, or None
    /// when the endpoint is unknown.
    pub(crate) fn frontend_endpoint_boot_id(
        &self,
        endpoint_id: &ClientEndpointId,
    ) -> Option<Option<String>> {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .map(|endpoint| {
                endpoint
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.boot_id.clone())
            })
    }

    pub(crate) fn frontend_endpoint_route(
        &self,
        endpoint_id: &ClientEndpointId,
        registry: &crate::client::endpoint::EndpointRegistry,
    ) -> Option<FrontendEndpointRoute<'_>> {
        self.endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
            .map(|endpoint| endpoint.frontend_route(registry))
    }

    /// Scalar-only fingerprint: no terminal locks, frame scans, allocation, or
    /// sorting. In-place acknowledgement rewrites bump
    /// `agent_projection_revision` because they leave snapshot identity intact.
    pub(crate) fn frontend_observation_fingerprint(
        &self,
        registry: &crate::client::endpoint::EndpointRegistry,
        frozen: bool,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.active_endpoint_id.hash(&mut hash);
        self.outer_focused.hash(&mut hash);
        frozen.hash(&mut hash);
        registry.active_surface_available().hash(&mut hash);
        std::mem::discriminant(&self.mode).hash(&mut hash);
        self.overlay.is_some().hash(&mut hash);
        self.frontend_input_pane().hash(&mut hash);
        self.agent_projection_revision.hash(&mut hash);
        for endpoint in &self.endpoints {
            endpoint.endpoint_id.hash(&mut hash);
            endpoint.label.hash(&mut hash);
            std::mem::discriminant(&endpoint.status).hash(&mut hash);
            endpoint.snapshot_generation.hash(&mut hash);
            endpoint
                .snapshot
                .as_ref()
                .map(|snapshot| (&snapshot.boot_id, snapshot.revision))
                .hash(&mut hash);
            registry
                .connection(&endpoint.endpoint_id)
                .map(|connection| connection.generation)
                .hash(&mut hash);
        }
        hash.finish()
    }

    /// The pane receiving ordinary typed input, or None while a mode or overlay
    /// owns the keyboard.
    fn frontend_input_pane(&self) -> Option<String> {
        match self.clipboard_image_target()? {
            crate::protocol::ClientClipboardImageTarget::Pane(pane) => Some(pane),
            crate::protocol::ClientClipboardImageTarget::Popup(_) => None,
            crate::protocol::ClientClipboardImageTarget::DirectTerminal => None,
        }
    }

    pub(crate) fn frontend_target_active(&self, target: &ClientEndpointFocusTarget) -> bool {
        let Some(snapshot) = self.snapshot.as_ref() else {
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

    pub(crate) fn frontend_snapshot(
        &self,
        registry: &crate::client::endpoint::EndpointRegistry,
        frozen: bool,
    ) -> Value {
        let endpoints = self
            .endpoints
            .iter()
            .map(|endpoint| {
                let status = match endpoint.status {
                    ClientEndpointStatus::Connecting => "connecting",
                    ClientEndpointStatus::Online => "online",
                    ClientEndpointStatus::Reconnecting => "reconnecting",
                    ClientEndpointStatus::Attention => "attention",
                    ClientEndpointStatus::Disabled => "disabled",
                };
                json!({
                    "endpoint_id": endpoint.endpoint_id.storage_key(),
                    "label": endpoint.label,
                    "status": status,
                    "boot_id": endpoint.snapshot.as_ref().map(|snapshot| &snapshot.boot_id),
                    "snapshot": endpoint.snapshot,
                })
            })
            .collect::<Vec<_>>();
        let active = self.active_endpoint_id.storage_key();
        let ready = registry.active_surface_available() && !frozen;
        let input_target = ready
            .then(|| self.frontend_input_pane())
            .flatten()
            .map(|pane| json!({"endpoint_id": active, "pane_id": pane}));
        json!({
            "focused": self.outer_focused,
            "input_ready": ready,
            "active_endpoint_id": active,
            "input_target": input_target,
            "endpoints": endpoints,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_identity_is_the_retained_snapshot_boot() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        assert_eq!(
            state.frontend_endpoint_boot_id(&ClientEndpointId::Local),
            Some(None)
        );
        assert_eq!(
            state.frontend_endpoint_boot_id(&ClientEndpointId::Ssh(
                crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef")
                    .unwrap()
            )),
            None
        );
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.boot_id = "boot-a".into();
        state.cache_endpoint_snapshot_inactive_for_generation(
            &ClientEndpointId::Local,
            3,
            Box::new(snapshot),
        );
        assert_eq!(
            state.frontend_endpoint_boot_id(&ClientEndpointId::Local),
            Some(Some("boot-a".into()))
        );
    }

    #[test]
    fn frontend_snapshot_carries_only_the_published_fields() {
        let state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        let registry = crate::client::endpoint::EndpointRegistry::empty();
        let snapshot = state.frontend_snapshot(&registry, false);
        let keys: Vec<_> = snapshot.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "active_endpoint_id",
                "endpoints",
                "focused",
                "input_ready",
                "input_target"
            ]
        );
        let endpoint = &snapshot["endpoints"][0];
        let keys: Vec<_> = endpoint.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            ["boot_id", "endpoint_id", "label", "snapshot", "status"]
        );
    }
}
