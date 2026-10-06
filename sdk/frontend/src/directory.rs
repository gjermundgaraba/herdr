//! Discovery of every live TUI in a socket directory, each with one push
//! subscription. Quiet subscriptions stay live until EOF; there is no
//! heartbeat. Every observed snapshot is delivered, including A→B→A, so route
//! guards see each invalidation; callers that only draw may coalesce.
use crate::{FrontendClient, Route, Snapshot};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// A captured destination: which TUI socket, which endpoint, and the server
/// boot whose pane ids were observed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientRoute {
    pub client_id: String,
    pub endpoint_id: String,
    pub boot_id: Option<String>,
    pub socket_path: PathBuf,
}

impl ClientRoute {
    pub fn wire(&self) -> Route {
        Route {
            endpoint_id: self.endpoint_id.clone(),
            boot_id: self.boot_id.clone(),
        }
    }

    pub fn client(&self) -> FrontendClient {
        FrontendClient::connect(&self.socket_path)
    }
}

/// One TUI's latest snapshot and the socket it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientState {
    pub socket_path: PathBuf,
    pub snapshot: Snapshot,
}

impl std::ops::Deref for ClientState {
    type Target = Snapshot;
    fn deref(&self) -> &Snapshot {
        &self.snapshot
    }
}

impl ClientState {
    pub fn route(&self, id: &str) -> Option<ClientRoute> {
        let route = self.snapshot.route(id)?;
        Some(ClientRoute {
            client_id: self.client_id.clone(),
            endpoint_id: route.endpoint_id,
            boot_id: route.boot_id,
            socket_path: self.socket_path.clone(),
        })
    }

    /// Ordinary input needs only the TUI; the endpoint is informational.
    pub fn input_route(&self) -> ClientRoute {
        let route = self
            .active_endpoint_id
            .as_deref()
            .and_then(|id| self.route(id));
        route.unwrap_or_else(|| ClientRoute {
            client_id: self.client_id.clone(),
            endpoint_id: self.active_endpoint_id.clone().unwrap_or_default(),
            boot_id: None,
            socket_path: self.socket_path.clone(),
        })
    }
}

/// Every live TUI, ordered by socket path.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    pub clients: Vec<ClientState>,
}

impl Model {
    /// The one TUI with terminal focus, if exactly one has it.
    pub fn foremost_client(&self) -> Option<&ClientState> {
        let mut focused = self.clients.iter().filter(|c| c.focused == Some(true));
        let first = focused.next()?;
        focused.next().is_none().then_some(first)
    }
}

/// The live TUIs after one observed change.
pub type Update = Vec<ClientState>;

/// Watches `directory` until `stopping` is set or `on_update` returns false.
/// Discovery rescans every 250 ms; pushed snapshots arrive at once.
pub fn spawn_updates(
    directory: PathBuf,
    mut on_update: impl FnMut(Update) -> bool + Send + 'static,
    stopping: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let (tx, rx) = mpsc::sync_channel(128);
        let mut workers = BTreeMap::new();
        let mut snapshots = BTreeMap::new();
        let mut next_scan = Instant::now();
        while !stopping.load(Ordering::Acquire) {
            if Instant::now() >= next_scan {
                let paths = crate::discover(&directory);
                workers.retain(
                    |path, (stop, handle): &mut (Arc<AtomicBool>, thread::JoinHandle<()>)| {
                        if !paths.contains(path) {
                            stop.store(true, Ordering::Release);
                            snapshots.remove(path);
                        }
                        !handle.is_finished()
                    },
                );
                for path in paths {
                    if workers.contains_key(&path) {
                        continue;
                    }
                    let stop = Arc::new(AtomicBool::new(false));
                    let worker_stop = stop.clone();
                    let stopping = stopping.clone();
                    let tx = tx.clone();
                    let worker_path = path.clone();
                    let handle = thread::spawn(move || {
                        watch(worker_path, tx, worker_stop, stopping);
                    });
                    workers.insert(path, (stop, handle));
                }
                if !on_update(snapshots.values().cloned().collect()) {
                    break;
                }
                next_scan = Instant::now() + Duration::from_millis(250);
            }
            if let Ok((path, owner, snapshot)) = rx.recv_timeout(Duration::from_millis(50)) {
                // A stopped owner cannot resurrect its observations. It is retained
                // until exit, so a replacement watcher cannot race its final EOF.
                if workers.get(&path).is_none_or(|(stop, _)| {
                    !Arc::ptr_eq(stop, &owner) || stop.load(Ordering::Acquire)
                }) {
                    continue;
                }
                if let Some(snapshot) = snapshot {
                    snapshots.insert(
                        path.clone(),
                        ClientState {
                            socket_path: path,
                            snapshot,
                        },
                    );
                } else {
                    snapshots.remove(&path);
                }
                if !on_update(snapshots.values().cloned().collect()) {
                    break;
                }
            }
        }
        for (stop, _) in workers.values() {
            stop.store(true, Ordering::Release);
        }
        drop(rx); // Release workers blocked on the bounded delivery queue.
        for (_, (_, handle)) in workers {
            let _ = handle.join();
        }
    })
}

type Observation = (PathBuf, Arc<AtomicBool>, Option<Snapshot>);

/// Subscribes to one TUI, reconnecting with backoff, and reports each
/// snapshot and each disconnect.
fn watch(
    path: PathBuf,
    tx: mpsc::SyncSender<Observation>,
    stop: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
) {
    let stopped = || stop.load(Ordering::Acquire) || stopping.load(Ordering::Acquire);
    let mut delay = Duration::from_millis(250);
    while !stopped() {
        if let Ok(mut subscription) = FrontendClient::connect(&path)
            .with_timeout(Duration::from_secs(1))
            .subscribe()
        {
            delay = Duration::from_millis(250);
            while !stopped() {
                match subscription.next_snapshot(Duration::from_millis(100)) {
                    Ok(Some(snapshot)) => {
                        if tx
                            .send((path.clone(), stop.clone(), Some(snapshot)))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(e) if e.code == "timeout" => continue,
                    _ => break,
                }
            }
        }
        if tx.send((path.clone(), stop.clone(), None)).is_err() {
            return;
        }
        let until = Instant::now() + delay;
        while !stopped() && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        delay = (delay * 2).min(Duration::from_secs(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::{fs::PermissionsExt, net::UnixListener},
    };

    #[test]
    fn discovery_delivers_bursts_quiet_connections_eof_and_reconnect() {
        let dir = std::env::temp_dir().join(format!("frontend-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("client.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (advance, commands) = mpsc::channel();
        let server = thread::spawn(move || {
            for round in 0..2 {
                listener.set_nonblocking(true).unwrap();
                let deadline = Instant::now() + Duration::from_secs(5);
                let stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "missing subscription");
                    thread::sleep(Duration::from_millis(10));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                writeln!(
                    reader.get_mut(),
                    "{}",
                    json!({"type":"hello","protocol":crate::PROTOCOL,"client_id":"client-a"})
                )
                .unwrap();
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&request).unwrap()["type"],
                    "subscribe"
                );
                for revision in 1..=3 {
                    let snapshot = json!({
                        "client_id": "client-a", "revision": revision + round * 3,
                        "focused": true, "input_ready": true, "active_endpoint_id": null,
                        "input_target": null, "endpoints": [],
                    });
                    writeln!(
                        reader.get_mut(),
                        "{}",
                        json!({"type":"snapshot","id":1,"snapshot":snapshot})
                    )
                    .unwrap();
                }
                commands.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });
        let stopping = Arc::new(AtomicBool::new(false));
        let (updates, received) = mpsc::channel();
        let worker = spawn_updates(
            dir.clone(),
            move |clients| updates.send(clients).is_ok(),
            stopping.clone(),
        );
        // Rescans repeat the current state, so only changes count.
        let mut last = 0;
        let mut next_revision = || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let clients: Update = received
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .unwrap();
                if let Some(client) = clients.first().filter(|client| client.revision != last) {
                    last = client.revision;
                    return last;
                }
            }
        };
        // A burst arrives whole and in order, not coalesced.
        assert_eq!(
            [next_revision(), next_revision(), next_revision()],
            [1, 2, 3]
        );
        // Several read timeouts are not EOF and must not clear a quiet client.
        thread::sleep(Duration::from_millis(350));
        for clients in received.try_iter() {
            assert!(!clients.is_empty());
        }
        advance.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let clients = received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if clients.is_empty() {
                break;
            }
        }
        assert_eq!(
            [next_revision(), next_revision(), next_revision()],
            [4, 5, 6]
        );
        stopping.store(true, Ordering::Release);
        advance.send(()).unwrap();
        server.join().unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
