//! Process-scoped frontend API for same-user tools on the TUI host. Protocol 7
//! is private and lockstep: consumers and this client ship together, with no
//! fallback to older versions.
//!
//! Only the Unix socket listener is platform specific; request handling and
//! reply settlement are plain client-loop logic.
use std::io;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

use super::endpoint::{ClientEndpointId, ClientEndpointStatus, EndpointRegistry, ProfileId};
use super::{
    shell::{
        ClientEndpointFocusTarget, ClientShellEndpointError, ClientShellInput, ClientShellState,
    },
    ClientLoopEvent,
};

const PROTOCOL: u32 = 7;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Cross-endpoint selects settle via the observed selection; this bounds the wait.
const SELECT_DEADLINE: Duration = Duration::from_secs(12);

/// Where a request is aimed: the endpoint and the server boot that produced
/// the ids the consumer captured. Pane, tab, and workspace ids are per-server
/// counters, so a different boot must reject rather than act on a reused id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Route {
    pub(super) endpoint_id: String,
    pub(super) boot_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Target {
    Workspace(String),
    Tab(String),
    Pane(String),
}
impl From<Target> for ClientEndpointFocusTarget {
    fn from(target: Target) -> Self {
        match target {
            Target::Workspace(id) => Self::Workspace(id),
            Target::Tab(id) => Self::Tab(id),
            Target::Pane(id) => Self::Pane(id),
        }
    }
}
impl Target {
    fn focus_method(&self) -> (&'static str, Value) {
        match self {
            Self::Workspace(id) => ("workspace.focus", json!({"workspace_id": id})),
            Self::Tab(id) => ("tab.focus", json!({"tab_id": id})),
            Self::Pane(id) => ("pane.focus", json!({"pane_id": id})),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Request {
    protocol: u32,
    id: u64,
    #[serde(flatten)]
    body: RequestBody,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum RequestBody {
    Snapshot {},
    Subscribe {},
    Select {
        route: Route,
        target: Target,
    },
    Input {
        text: Option<String>,
        keys: Option<Vec<String>>,
    },
    /// Any endpoint method the server advertises on the client command lane.
    Call {
        route: Route,
        method: String,
        #[serde(default)]
        params: Value,
    },
}

pub(super) struct Event {
    pub(super) request: Request,
    pub(super) reply: oneshot::Sender<Value>,
}

/// Validated intent, dispatched in the same event-loop turn before acknowledgment.
pub(super) enum Dispatch {
    /// Dispatch through the ordinary input pipeline; the loop replies after applying.
    Input {
        id: u64,
        reply: oneshot::Sender<Value>,
        input: ClientShellInput,
    },
    /// The endpoint is not active: start a native activation carrying the target.
    /// The reply lives in the api's pending selects and settles from observation.
    Select {
        endpoint: ClientEndpointId,
        target: ClientEndpointFocusTarget,
    },
}

/// A frontend reply waiting on an endpoint command result. The shell keeps it
/// beside its own pending requests and settles it through [`FrontendReply::settle`]
/// when the result, expiry, or cancellation arrives.
#[derive(Debug)]
pub(crate) struct FrontendReply {
    id: u64,
    sender: oneshot::Sender<Value>,
}

impl FrontendReply {
    pub(crate) fn settle(
        self,
        result: &Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) {
        let response = match result {
            Ok(result) => success(self.id, json!(result)),
            Err(rejected) => {
                let code = match rejected.code.as_deref() {
                    Some("endpoint_cancelled") => "cancelled",
                    Some(code) => code,
                    None => "endpoint_error",
                };
                error(Some(self.id), code, &rejected.message)
            }
        };
        let _ = self.sender.send(response);
    }
}

struct PendingSelect {
    id: u64,
    sender: oneshot::Sender<Value>,
    endpoint: ClientEndpointId,
    target: ClientEndpointFocusTarget,
    queued_at: Instant,
}

pub(super) struct FrontendApi {
    client_id: String,
    listener: Option<tokio::task::JoinHandle<()>>,
    socket_path: Option<PathBuf>,
    revision: u64,
    snapshots: watch::Sender<Arc<Value>>,
    subscribers: Arc<AtomicUsize>,
    previous_observation: Option<u64>,
    pending_selects: Vec<PendingSelect>,
    next_request: u64,
}

impl Drop for FrontendApi {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.take() {
            listener.abort();
        }
        if let Some(path) = self.socket_path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(super) fn error(id: Option<u64>, code: &str, message: &str) -> Value {
    json!({"type":"error","id":id,"error":{"code":code,"message":message}})
}
pub(super) fn success(id: u64, result: Value) -> Value {
    json!({"type":"reply","id":id,"result":result})
}

impl FrontendApi {
    #[cfg(not(unix))]
    pub(super) fn start(_tx: tokio::sync::mpsc::Sender<ClientLoopEvent>) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the frontend socket is Unix only",
        ))
    }

    #[cfg(unix)]
    pub(super) fn start(tx: tokio::sync::mpsc::Sender<ClientLoopEvent>) -> io::Result<Self> {
        let client_id = ProfileId::generate().as_str().to_owned();
        let (socket, socket_path) = listener::bind(&client_id)?;
        let (snapshots, current) = watch::channel(Arc::new(Value::Null));
        let subscribers = Arc::new(AtomicUsize::new(0));
        let demand = subscribers.clone();
        let identity = client_id.clone();
        let listener = tokio::spawn(async move {
            loop {
                match socket.accept().await {
                    Ok((stream, _)) => {
                        let (tx, current, identity, demand) = (
                            tx.clone(),
                            current.clone(),
                            identity.clone(),
                            demand.clone(),
                        );
                        tokio::spawn(async move {
                            if let Err(error) =
                                listener::serve(stream, tx, current, identity, demand).await
                            {
                                tracing::debug!(%error, "frontend connection closed");
                            }
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "frontend accept failed; retrying");
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                }
            }
        });
        Ok(Self {
            client_id,
            listener: Some(listener),
            socket_path: Some(socket_path),
            revision: 0,
            snapshots,
            subscribers,
            previous_observation: None,
            pending_selects: Vec::new(),
            next_request: 0,
        })
    }

    /// Called at the loop boundary, including turns ending in `continue`, never by rendering.
    pub(super) fn publish(
        &mut self,
        shell: &ClientShellState,
        frozen: bool,
        registry: &EndpointRegistry,
    ) {
        // Snapshot requests explicitly refresh the inventory in the client loop;
        // only subscriptions keep it hot.
        if self.subscribers.load(Ordering::Acquire) == 0 {
            return;
        }
        self.refresh_snapshot(shell, frozen, registry);
    }

    fn refresh_snapshot(
        &mut self,
        shell: &ClientShellState,
        frozen: bool,
        registry: &EndpointRegistry,
    ) {
        let observation = Some(shell.frontend_observation_fingerprint(registry, frozen));
        if observation == self.previous_observation {
            return;
        }
        self.previous_observation = observation;
        self.revision += 1;
        let mut snapshot = shell.frontend_snapshot(registry, frozen);
        snapshot["client_id"] = json!(self.client_id);
        snapshot["revision"] = json!(self.revision);
        self.snapshots.send_replace(Arc::new(snapshot));
    }

    /// Resolve a route to an endpoint whose retained snapshot still carries the
    /// captured boot identity.
    fn resolve_route(
        route: &Route,
        shell: &ClientShellState,
    ) -> Result<ClientEndpointId, (&'static str, &'static str)> {
        let id = parse_endpoint(&route.endpoint_id)
            .ok_or(("unknown_endpoint", "invalid endpoint identity"))?;
        let boot_id = shell.frontend_endpoint_boot_id(&id).ok_or((
            "unknown_endpoint",
            "endpoint is not a member of this frontend",
        ))?;
        if boot_id != route.boot_id {
            return Err((
                "stale_route",
                "endpoint identity changed; refresh the snapshot and retry once",
            ));
        }
        Ok(id)
    }

    /// Validate and enqueue one request on the endpoint command lane. The shell
    /// settles the reply from the endpoint result or its cancellation.
    #[allow(clippy::too_many_arguments)] // One validation path for selects and calls.
    fn enqueue_endpoint_request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        endpoint_id: ClientEndpointId,
        shell: &mut ClientShellState,
        registry: &mut EndpointRegistry,
        commands: &mut super::endpoint_commands::EndpointCommands,
        reply: oneshot::Sender<Value>,
    ) {
        let Some(route) = shell.frontend_endpoint_route(&endpoint_id, registry) else {
            let _ = reply.send(error(
                Some(id),
                "unknown_endpoint",
                "endpoint is not a member of this frontend",
            ));
            return;
        };
        if route.status != ClientEndpointStatus::Online {
            let _ = reply.send(error(
                Some(id),
                "endpoint_unavailable",
                "endpoint is not online",
            ));
            return;
        }
        let Some(boot_id) = route.boot_id.map(str::to_owned) else {
            let _ = reply.send(error(
                Some(id),
                "endpoint_not_ready",
                "endpoint has no server identity yet; retry after its snapshot arrives",
            ));
            return;
        };
        let generation = route.generation;
        if !registry
            .connection(&endpoint_id)
            .is_some_and(|connection| connection.negotiation.supports_method(method))
        {
            let _ = reply.send(error(
                Some(id),
                "unsupported_method",
                "endpoint does not advertise this method",
            ));
            return;
        }
        self.next_request += 1;
        let request_id = format!("frontend-api:{}:{}", self.client_id, self.next_request);
        let request = match serde_json::from_value::<crate::api::schema::Request>(
            json!({"id":request_id,"method":method,"params":params}),
        ) {
            Ok(request) => request,
            Err(_) => {
                let _ = reply.send(error(
                    Some(id),
                    "invalid_params",
                    "method or parameters are invalid",
                ));
                return;
            }
        };
        shell.register_frontend_request(
            request_id,
            boot_id.clone(),
            method.to_owned(),
            FrontendReply { id, sender: reply },
        );
        commands.enqueue(endpoint_id.clone(), generation, boot_id, Box::new(request));
        // A request that could not be sent (stale generation, encode failure,
        // send rejection) is cancelled immediately; do not wait out the exchange.
        for cancelled in commands.send_next(&endpoint_id, registry) {
            shell.cancel_endpoint_request(&cancelled);
        }
    }

    pub(super) fn handle(
        &mut self,
        event: Event,
        shell: &mut ClientShellState,
        frozen: bool,
        endpoints: &mut EndpointRegistry,
        commands: &mut super::endpoint_commands::EndpointCommands,
    ) -> Option<Dispatch> {
        let id = event.request.id;
        match event.request.body {
            RequestBody::Snapshot { .. } => {
                self.refresh_snapshot(shell, frozen, endpoints);
                // The socket borrows the latest watch value for serialization
                // instead of copying the inventory into a reply.
                let _ = event.reply.send(success(id, Value::Null));
            }
            RequestBody::Subscribe { .. } => {
                let _ = event.reply.send(error(
                    Some(id),
                    "invalid_request",
                    "observation request reached mutation dispatch",
                ));
            }
            RequestBody::Input { text, keys } => {
                let data = match input_events(text, keys) {
                    Ok(data) => data,
                    Err(message) => {
                        let _ = event
                            .reply
                            .send(error(Some(id), "invalid_params", &message));
                        return None;
                    }
                };
                return Some(Dispatch::Input {
                    id,
                    reply: event.reply,
                    input: shell.handle_raw_events(data),
                });
            }
            RequestBody::Select { route, target } => {
                let endpoint_id = match Self::resolve_route(&route, shell) {
                    Ok(id) => id,
                    Err((code, message)) => {
                        let _ = event.reply.send(error(Some(id), code, message));
                        return None;
                    }
                };
                if *endpoints.active_id() == endpoint_id {
                    let (method, params) = target.focus_method();
                    self.enqueue_endpoint_request(
                        id,
                        method,
                        params,
                        endpoint_id,
                        shell,
                        endpoints,
                        commands,
                        event.reply,
                    );
                } else {
                    let target: ClientEndpointFocusTarget = target.into();
                    self.pending_selects.push(PendingSelect {
                        id,
                        sender: event.reply,
                        endpoint: endpoint_id.clone(),
                        target: target.clone(),
                        queued_at: Instant::now(),
                    });
                    return Some(Dispatch::Select {
                        endpoint: endpoint_id,
                        target,
                    });
                }
            }
            RequestBody::Call {
                route,
                method,
                params,
            } => {
                let endpoint_id = match Self::resolve_route(&route, shell) {
                    Ok(id) => id,
                    Err((code, message)) => {
                        let _ = event.reply.send(error(Some(id), code, message));
                        return None;
                    }
                };
                if *endpoints.active_id() != endpoint_id
                    || !endpoints.active_surface_available()
                    || frozen
                {
                    let _ = event.reply.send(error(
                        Some(id),
                        "inactive_endpoint",
                        "endpoint must already be active with an available lease",
                    ));
                    return None;
                }
                self.enqueue_endpoint_request(
                    id,
                    &method,
                    params,
                    endpoint_id,
                    shell,
                    endpoints,
                    commands,
                    event.reply,
                );
            }
        }
        None
    }

    /// Settle cross-endpoint selects from observed state. Called once per loop
    /// boundary; replies only on a confirmed target or the deadline.
    pub(super) fn settle_selects(&mut self, shell: &ClientShellState, now: Instant) {
        let mut keep = Vec::new();
        for pending in self.pending_selects.drain(..) {
            let response = if shell.active_endpoint_id == pending.endpoint
                && shell.frontend_target_active(&pending.target)
            {
                Some(success(pending.id, json!({"ok": true})))
            } else if now.duration_since(pending.queued_at) >= SELECT_DEADLINE {
                Some(error(
                    Some(pending.id),
                    "timeout",
                    "navigation did not settle; outcome is unknown; do not replay",
                ))
            } else {
                None
            };
            match response {
                Some(response) => {
                    let _ = pending.sender.send(response);
                }
                None => keep.push(pending),
            }
        }
        self.pending_selects = keep;
    }
}

struct InventorySubscription(Arc<AtomicUsize>);
impl InventorySubscription {
    fn new(subscribers: Arc<AtomicUsize>) -> Self {
        subscribers.fetch_add(1, Ordering::AcqRel);
        Self(subscribers)
    }
}
impl Drop for InventorySubscription {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn input_events(
    text: Option<String>,
    keys: Option<Vec<String>>,
) -> Result<Vec<crate::raw_input::RawInputEvent>, String> {
    match (text, keys) {
        (Some(text), None) => Ok(crate::raw_input::parse_raw_input_bytes_sync(
            text.as_bytes(),
        )),
        (None, Some(keys)) => keys
            .into_iter()
            .map(|key| {
                let event = crate::app::api_helpers::parse_api_key(&key)
                    .ok_or_else(|| format!("unknown key: {key}"))?;
                Ok(crate::raw_input::RawInputEvent::Key(event.into()))
            })
            .collect(),
        _ => Err("provide exactly one of text or keys".into()),
    }
}

fn parse_endpoint(value: &str) -> Option<ClientEndpointId> {
    if value == "local" {
        Some(ClientEndpointId::Local)
    } else {
        ProfileId::parse(value.strip_prefix("ssh:")?)
            .ok()
            .map(ClientEndpointId::Ssh)
    }
}

async fn exchange(tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>, request: Request) -> Value {
    let id = request.id;
    let (reply, response) = oneshot::channel();
    match tx.try_send(ClientLoopEvent::FrontendApi(Event { request, reply })) {
        Ok(()) => match tokio::time::timeout(Duration::from_secs(65), response).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => error(
                Some(id),
                "client_unavailable",
                "client dropped the reply; mutation outcome is unknown; do not replay",
            ),
            Err(_) => error(
                Some(id),
                "timeout",
                "client reply timed out; mutation outcome is unknown; do not replay",
            ),
        },
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            error(Some(id), "busy", "client event queue is full")
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => error(
            Some(id),
            "client_unavailable",
            "client event loop is unavailable; request was not submitted",
        ),
    }
}

#[derive(Serialize)]
struct SnapshotEnvelope<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: u64,
    snapshot: &'a Value,
}

impl<'a> SnapshotEnvelope<'a> {
    fn new(id: u64, snapshot: &'a Value) -> Self {
        Self {
            kind: "snapshot",
            id,
            snapshot,
        }
    }
}

/// The Unix socket: an owner-only directory holding one 0600 socket per TUI.
#[cfg(unix)]
mod listener {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};

    pub(super) fn bind(client_id: &str) -> io::Result<(UnixListener, PathBuf)> {
        let dir = std::env::var_os("HERDR_CLIENT_API_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // SAFETY: geteuid has no preconditions.
                PathBuf::from(format!("/tmp/herdr-clients-{}", unsafe { libc::geteuid() }))
            });
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let path = dir.join(format!("{client_id}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        crate::ipc::restrict_socket_permissions(&path, 0o600)?;
        listener.set_nonblocking(true)?;
        Ok((UnixListener::from_std(listener)?, path))
    }

    fn encode_message(value: &impl Serialize) -> io::Result<Vec<u8>> {
        let mut message = serde_json::to_vec(value)?;
        message.push(b'\n');
        Ok(message)
    }

    async fn write_message(
        stream: &mut (impl tokio::io::AsyncWrite + Unpin),
        value: &impl Serialize,
    ) -> io::Result<()> {
        tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(&encode_message(value)?),
        )
        .await
        .map_err(io::Error::other)?
    }

    pub(super) async fn serve(
        stream: UnixStream,
        tx: tokio::sync::mpsc::Sender<ClientLoopEvent>,
        current: watch::Receiver<Arc<Value>>,
        client_id: String,
        subscribers: Arc<AtomicUsize>,
    ) -> io::Result<()> {
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        write_message(
            &mut write,
            &json!({"type":"hello","protocol":PROTOCOL,"client_id":client_id}),
        )
        .await?;
        loop {
            let mut line = Vec::new();
            match (&mut reader)
                .take((MAX_REQUEST_BYTES + 1) as u64)
                .read_until(b'\n', &mut line)
                .await?
            {
                0 => return Ok(()),
                _ if line.len() > MAX_REQUEST_BYTES => {
                    write_message(
                        &mut write,
                        &error(None, "request_too_large", "request exceeds one MiB"),
                    )
                    .await?;
                    return Ok(());
                }
                _ => {}
            }
            let request = match serde_json::from_slice::<Request>(&line) {
                Ok(request) => request,
                Err(_) => {
                    let id = serde_json::from_slice::<Value>(&line)
                        .ok()
                        .and_then(|value| value["id"].as_u64());
                    write_message(
                        &mut write,
                        &error(id, "invalid_request", "invalid frontend request"),
                    )
                    .await?;
                    continue;
                }
            };
            let (protocol, id) = (request.protocol, request.id);
            if protocol != PROTOCOL {
                write_message(
                    &mut write,
                    &error(
                        Some(id),
                        "unsupported_protocol",
                        "expected frontend protocol 7",
                    ),
                )
                .await?;
                continue;
            }
            match &request.body {
                RequestBody::Subscribe { .. } => {
                    // A push subscription owns this connection. EOF or extra input cancels
                    // registration and delivery without a monitor thread or polling.
                    return tokio::select! {
                        _ = reader.fill_buf() => Ok(()),
                        result = async {
                            let _demand = InventorySubscription::new(subscribers);
                            let response = exchange(&tx, Request { protocol, id, body: RequestBody::Snapshot {} }).await;
                            if response["type"] != "reply" {
                                return write_message(&mut write, &response).await;
                            }
                            let mut current = current;
                            loop {
                                let snapshot = current.borrow_and_update().clone();
                                write_message(&mut write, &SnapshotEnvelope::new(id, &snapshot)).await?;
                                if current.changed().await.is_err() {
                                    return Ok(());
                                }
                            }
                        } => result,
                    };
                }
                _ => {
                    let snapshot_request = matches!(request.body, RequestBody::Snapshot { .. });
                    let response = exchange(&tx, request).await;
                    if snapshot_request && response["type"] == "reply" {
                        let snapshot = current.borrow().clone();
                        write_message(&mut write, &SnapshotEnvelope::new(id, &snapshot)).await?;
                    } else {
                        write_message(&mut write, &response).await?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::shell::{ClientShellConfig, ClientShellState};
    use crate::config::Config;

    fn shell() -> ClientShellState {
        ClientShellState::new(ClientShellConfig::from_config(&Config::default()))
    }

    fn test_api() -> FrontendApi {
        let (snapshots, _) = watch::channel(Arc::new(Value::Null));
        FrontendApi {
            client_id: "test-client".into(),
            listener: None,
            socket_path: None,
            revision: 0,
            snapshots,
            subscribers: Arc::new(AtomicUsize::new(0)),
            previous_observation: None,
            pending_selects: Vec::new(),
            next_request: 0,
        }
    }

    fn ssh_id() -> ClientEndpointId {
        ClientEndpointId::Ssh(ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap())
    }

    fn request(id: u64, body: Value) -> Request {
        let mut value = body;
        value["protocol"] = json!(PROTOCOL);
        value["id"] = json!(id);
        serde_json::from_value(value).expect("valid request")
    }

    fn handle(
        api: &mut FrontendApi,
        shell: &mut ClientShellState,
        id: u64,
        body: Value,
    ) -> (Option<Dispatch>, oneshot::Receiver<Value>) {
        let mut endpoints = EndpointRegistry::empty();
        let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
        let (sender, receiver) = oneshot::channel();
        let event = Event {
            request: request(id, body),
            reply: sender,
        };
        let dispatch = api.handle(event, shell, false, &mut endpoints, &mut commands);
        (dispatch, receiver)
    }

    #[test]
    fn error_and_success_payloads_match_the_wire_contract() {
        let value = error(Some(7), "stale_route", "refresh and retry");
        assert_eq!(value["type"], "error");
        assert_eq!(value["id"], 7);
        assert_eq!(value["error"]["code"], "stale_route");
        let value = success(8, json!({"ok": true}));
        assert_eq!(value["type"], "reply");
        assert_eq!(value["id"], 8);
        assert_eq!(value["result"]["ok"], true);
    }

    #[test]
    fn input_events_require_exactly_one_of_text_or_keys() {
        assert!(input_events(Some("hello".into()), None).is_ok());
        assert!(input_events(None, Some(vec!["ctrl+a".into()])).is_ok());
        assert!(input_events(Some("x".into()), Some(vec!["a".into()])).is_err());
        assert!(input_events(None, None).is_err());
        assert!(input_events(None, Some(vec!["not-a-key".into()])).is_err());
    }

    #[test]
    fn endpoint_identity_parses_local_and_ssh() {
        assert_eq!(parse_endpoint("local"), Some(ClientEndpointId::Local));
        assert!(parse_endpoint("nostr").is_none());
        let profile = "0123456789abcdef0123456789abcdef";
        assert_eq!(parse_endpoint(&format!("ssh:{profile}")), Some(ssh_id()));
    }

    #[test]
    fn settle_selects_reports_deadline_and_keeps_fresh_until_confirmed() {
        let mut api = test_api();
        let (sender, receiver) = oneshot::channel();
        api.pending_selects.push(PendingSelect {
            id: 1,
            sender,
            endpoint: ssh_id(),
            target: ClientEndpointFocusTarget::Pane("p1".into()),
            queued_at: Instant::now() - Duration::from_secs(15),
        });
        let shell = shell();
        api.settle_selects(&shell, Instant::now());
        let reply = receiver.blocking_recv().unwrap();
        assert_eq!(reply["error"]["code"], "timeout");

        let mut api = test_api();
        let (sender, mut receiver) = oneshot::channel();
        api.pending_selects.push(PendingSelect {
            id: 2,
            sender,
            endpoint: ssh_id(),
            target: ClientEndpointFocusTarget::Tab("t1".into()),
            queued_at: Instant::now() - Duration::from_secs(1),
        });
        api.settle_selects(&shell, Instant::now());
        assert_eq!(api.pending_selects.len(), 1);
        assert_eq!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        );
    }

    fn register(
        shell: &mut ClientShellState,
        request_id: &str,
        id: u64,
    ) -> oneshot::Receiver<Value> {
        let (sender, receiver) = oneshot::channel();
        shell.register_frontend_request(
            request_id.into(),
            "boot".into(),
            "agent.prompt".into(),
            FrontendReply { id, sender },
        );
        receiver
    }

    #[test]
    fn shell_results_settle_calls_and_cancellations_reply_unknown_outcome() {
        let mut shell = shell();
        let receiver = register(&mut shell, "frontend-api:test:1", 7);
        shell.handle_endpoint_result(
            "boot",
            "frontend-api:test:1",
            Ok(crate::api::schema::ResponseResult::Pong {
                version: "0.9.1".into(),
                protocol: 5,
                capabilities: None,
            }),
        );
        let reply = receiver.blocking_recv().unwrap();
        assert_eq!(reply["type"], "reply");
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["version"], "0.9.1");

        let receiver = register(&mut shell, "frontend-api:test:2", 9);
        shell.cancel_endpoint_request("frontend-api:test:2");
        assert_eq!(
            receiver.blocking_recv().unwrap()["error"]["code"],
            "cancelled"
        );
    }

    #[test]
    fn snapshot_request_publishes_inventory_and_replies() {
        let mut api = test_api();
        let mut shell = shell();
        let (dispatch, receiver) = handle(&mut api, &mut shell, 3, json!({"type":"snapshot"}));
        assert!(dispatch.is_none());
        assert_eq!(receiver.blocking_recv().unwrap()["type"], "reply");
        let snapshot = api.snapshots.borrow().clone();
        assert_eq!(snapshot["client_id"], "test-client");
        assert_eq!(snapshot["revision"], 1);
        assert_eq!(snapshot["active_endpoint_id"], "local");
    }

    #[test]
    fn subscriptions_never_reach_mutation_dispatch() {
        let mut api = test_api();
        let mut shell = shell();
        let (dispatch, receiver) = handle(&mut api, &mut shell, 4, json!({"type":"subscribe"}));
        assert!(dispatch.is_none());
        assert_eq!(
            receiver.blocking_recv().unwrap()["error"]["code"],
            "invalid_request"
        );
    }

    #[test]
    fn input_dispatch_returns_a_pipeline_outcome_for_the_loop() {
        let mut api = test_api();
        let mut shell = shell();
        let (dispatch, mut receiver) = handle(
            &mut api,
            &mut shell,
            11,
            json!({"type":"input","keys":["ctrl+a"]}),
        );
        let Some(Dispatch::Input { id, input, .. }) = dispatch else {
            panic!("expected input dispatch");
        };
        assert_eq!(id, 11);
        assert_eq!(input.actions.len(), 0);
        // The loop owns the reply; it is sent after applying the outcome.
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn select_and_call_on_unknown_endpoint_report_unknown_endpoint() {
        let mut api = test_api();
        let mut shell = shell();
        let route = json!({"endpoint_id":"ssh:0123456789abcdef0123456789abcdef","boot_id":"boot"});
        for body in [
            json!({"type":"select","route":route,"target":{"pane":"p1"}}),
            json!({"type":"call","route":route,"method":"agent.prompt","params":{"target":"p1","text":"hi"}}),
        ] {
            let (dispatch, receiver) = handle(&mut api, &mut shell, 13, body);
            assert!(dispatch.is_none());
            assert_eq!(
                receiver.blocking_recv().unwrap()["error"]["code"],
                "unknown_endpoint"
            );
        }
    }

    #[test]
    fn stale_boot_identity_is_rejected_before_dispatch() {
        let mut api = test_api();
        let mut shell = shell();
        let (dispatch, receiver) = handle(
            &mut api,
            &mut shell,
            15,
            json!({"type":"call","route":{"endpoint_id":"local","boot_id":"rebooted"},
                "method":"pane.focus_direction","params":{"pane_id":"p1","direction":"left"}}),
        );
        assert!(dispatch.is_none());
        assert_eq!(
            receiver.blocking_recv().unwrap()["error"]["code"],
            "stale_route"
        );
    }

    #[test]
    fn call_on_the_inactive_local_endpoint_is_rejected_before_enqueue() {
        let mut api = test_api();
        let mut shell = shell();
        let mut endpoints = EndpointRegistry::empty();
        let mut commands = crate::client::endpoint_commands::EndpointCommands::default();
        let (sender, receiver) = oneshot::channel();
        let event = Event {
            request: request(
                14,
                json!({"type":"call",
                    "route":{"endpoint_id":"local","boot_id":null},
                    "method":"pane.focus_direction","params":{"pane_id":"p1","direction":"left"}}),
            ),
            reply: sender,
        };
        assert!(api
            .handle(event, &mut shell, true, &mut endpoints, &mut commands)
            .is_none());
        assert_eq!(
            receiver.blocking_recv().unwrap()["error"]["code"],
            "inactive_endpoint"
        );
    }
}
