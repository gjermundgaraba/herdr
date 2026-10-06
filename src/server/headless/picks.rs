//! `ui.pick`: plugin pickers shown natively on one TUI client. The waiting
//! API caller holds the request open; the picker's client answers with
//! `ui.pick.resolve`, and a closed picker or a departed client cancels it.

use super::HeadlessServer;
use crate::api;
use crate::api::schema::{
    ErrorBody, ErrorResponse, Method, ResponseResult, SuccessResponse, UiPickOutcome,
};
use crate::protocol::endpoint::{pick_close_message, pick_open_message, EndpointPickOpen};

pub(super) struct PendingPick {
    client_id: u64,
    respond_to: std::sync::mpsc::Sender<String>,
}

fn success(id: String, result: ResponseResult) -> String {
    serde_json::to_string(&SuccessResponse { id, result }).unwrap_or_else(|_| "{}".to_owned())
}

fn error(id: String, code: &str, message: &str) -> String {
    serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    })
    .unwrap_or_else(|_| "{}".to_owned())
}

impl HeadlessServer {
    /// Handles `ui.pick*` methods and returns the message for any other method.
    pub(super) fn handle_ui_pick_request(
        &mut self,
        msg: api::ApiRequestMessage,
    ) -> Option<api::ApiRequestMessage> {
        let id = msg.request.id.clone();
        let response = match &msg.request.method {
            Method::UiPick(params) => {
                let Some(client_id) = self.pick_client(params.client_id) else {
                    let _ = msg.respond_to.send(error(
                        id,
                        "no_client",
                        "no TUI client is attached to show the picker",
                    ));
                    return None;
                };
                if self
                    .pending_picks
                    .values()
                    .any(|pick| pick.client_id == client_id)
                {
                    let _ = msg.respond_to.send(error(
                        id,
                        "ui_busy",
                        "that client is already showing a picker",
                    ));
                    return None;
                }
                let open = EndpointPickOpen {
                    pick_id: id.clone(),
                    title: params.title.clone(),
                    items: params.items.clone(),
                    selected: params.selected.clone(),
                    create: params.create.clone(),
                };
                let sent = pick_open_message(&open)
                    .is_ok_and(|message| self.send_to_client(client_id, message));
                if !sent {
                    let _ = msg.respond_to.send(error(
                        id,
                        "no_client",
                        "the picker's client went away",
                    ));
                    return None;
                }
                self.pending_picks.insert(
                    id,
                    PendingPick {
                        client_id,
                        respond_to: msg.respond_to,
                    },
                );
                return None;
            }
            Method::UiPickResolve(params) => {
                let owned = self
                    .pending_picks
                    .get(&params.pick_id)
                    .is_some_and(|pick| Some(pick.client_id) == self.app.invoking_client_id);
                if owned {
                    self.finish_pick(&params.pick_id, params.outcome.clone());
                    success(id, ResponseResult::Ok {})
                } else {
                    error(
                        id,
                        "pick_not_found",
                        "no open picker with that id on this client",
                    )
                }
            }
            Method::UiPickClose(params) => {
                if let Some(client_id) = self.finish_pick(&params.pick_id, UiPickOutcome::Cancelled)
                {
                    if let Ok(message) = pick_close_message(&params.pick_id) {
                        self.send_to_client(client_id, message);
                    }
                    success(id, ResponseResult::Ok {})
                } else {
                    error(id, "pick_not_found", "no open picker with that id")
                }
            }
            _ => return Some(msg),
        };
        let _ = msg.respond_to.send(response);
        None
    }

    /// Answers the waiting caller and returns the picker's client.
    fn finish_pick(&mut self, pick_id: &str, outcome: UiPickOutcome) -> Option<u64> {
        let pick = self.pending_picks.remove(pick_id)?;
        let _ = pick.respond_to.send(success(
            pick_id.to_owned(),
            ResponseResult::UiPicked { outcome },
        ));
        Some(pick.client_id)
    }

    /// Cancels the pickers a departing client was showing.
    pub(super) fn cancel_client_picks(&mut self, client_id: u64) {
        let pick_ids = self
            .pending_picks
            .iter()
            .filter(|(_, pick)| pick.client_id == client_id)
            .map(|(pick_id, _)| pick_id.clone())
            .collect::<Vec<_>>();
        for pick_id in pick_ids {
            self.finish_pick(&pick_id, UiPickOutcome::Cancelled);
        }
    }

    /// The requested client when it can show a picker, else the foreground one.
    fn pick_client(&self, requested: Option<u64>) -> Option<u64> {
        let can_show = |client_id: &u64| {
            self.clients
                .get(client_id)
                .is_some_and(|client| client.is_active_shell_client())
        };
        requested
            .filter(can_show)
            .or(self.foreground_client_id.filter(can_show))
    }
}
