//! Attached clients as tools see them: the report each client sends about its
//! Agents list (`client.report`), `client.list`, and the `client.changed`
//! event. A report lives in memory only and goes when its client does.

use std::collections::HashMap;

use super::HeadlessServer;
use crate::api;
use crate::api::schema::{
    ClientInfo, ClientReport, EventData, EventEnvelope, EventKind, Method, ResponseResult,
};
use crate::app::{encode_error, encode_success};

/// Latest report per client id.
#[derive(Default)]
pub(super) struct ClientReports(HashMap<u64, ClientReport>);

impl ClientReports {
    #[cfg(test)]
    pub(super) fn get(&self, client_id: u64) -> Option<&ClientReport> {
        self.0.get(&client_id)
    }

    /// Whether the client has sent a report since it attached.
    pub(super) fn contains(&self, client_id: u64) -> bool {
        self.0.contains_key(&client_id)
    }
}

impl HeadlessServer {
    /// Handles the public `client.*` methods and returns the message for any
    /// other method.
    pub(super) fn handle_client_api_request(
        &mut self,
        msg: api::ApiRequestMessage,
    ) -> Option<api::ApiRequestMessage> {
        let id = msg.request.id.clone();
        let response = match &msg.request.method {
            Method::ClientList(_) => encode_success(
                id,
                ResponseResult::ClientList {
                    boot_id: self.client_shell_boot_id.clone(),
                    clients: self.client_infos(),
                },
            ),
            Method::ClientJump(_) => {
                self.start_client_jump(msg);
                return None;
            }
            Method::ClientReport(_) | Method::ClientJumpResolve(_) => encode_error(
                id,
                "connection_local_only",
                "this method is only available through a client shell endpoint",
            ),
            _ => return Some(msg),
        };
        let _ = msg.respond_to.send(response);
        None
    }

    /// Stores `client.report` from the client lane. An identical report is
    /// accepted without notifying subscribers.
    pub(super) fn accept_client_report(
        &mut self,
        client_id: u64,
        mut report: ClientReport,
    ) -> Result<ResponseResult, (&'static str, String)> {
        report
            .normalize()
            .map_err(|err| ("invalid_params", err.message().to_owned()))?;
        if self.client_reports.0.get(&client_id) != Some(&report) {
            self.client_reports.0.insert(client_id, report);
            self.emit_client_changed(client_id);
        }
        Ok(ResponseResult::Ok {})
    }

    /// Drops a departing client's report and tells subscribers it detached.
    pub(super) fn forget_client_report(&mut self, client_id: u64, was_shell_client: bool) {
        self.client_reports.0.remove(&client_id);
        if was_shell_client {
            self.emit_client_changed(client_id);
        }
    }

    /// Pushes `client.changed`. The event names the client only, so the event
    /// history never holds copies of reports; subscribers read `client.list`.
    pub(super) fn emit_client_changed(&self, client_id: u64) {
        self.app.event_hub.push(EventEnvelope {
            event: EventKind::ClientChanged,
            data: EventData::ClientChanged { client_id },
        });
    }

    /// Every attached shell client, lowest id first.
    fn client_infos(&self) -> Vec<ClientInfo> {
        let mut clients = self
            .clients
            .iter()
            .filter(|(_, client)| client.is_shell_client())
            .map(|(&client_id, _)| ClientInfo {
                client_id,
                report: self.client_reports.0.get(&client_id).cloned(),
            })
            .collect::<Vec<_>>();
        clients.sort_unstable_by_key(|client| client.client_id);
        clients
    }
}
