//! `ui.pick`: the API connection waits while a client shows the picker.
//! The client side is `src/client/shell/picks.rs`, the server side
//! `src/server/headless/picks.rs`.

use super::*;

static NEXT_PICK_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Holds a `ui.pick` request open until its client answers. The picker is
/// keyed by a server-unique id, because caller request ids may collide; a
/// caller that disconnects first closes its picker.
pub(in crate::api) fn wait_for_pick(
    request_id: String,
    params: crate::api::schema::UiPickParams,
    stream: &mut LocalStream,
    api_tx: &ApiRequestSender,
    running: &Arc<AtomicBool>,
) -> std::io::Result<Option<String>> {
    let pick_id = format!(
        "pick-{}",
        NEXT_PICK_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    let dispatched = api_tx.send(crate::api::ApiRequestMessage {
        request: Request {
            id: pick_id.clone(),
            method: Method::UiPick(params),
        },
        respond_to,
        response_write_complete: None,
    });
    if dispatched.is_err() {
        return Ok(Some(error_json(
            request_id,
            "server_unavailable",
            "failed to dispatch request",
        )));
    }
    loop {
        match response_rx.recv_timeout(CONNECTION_POLL_INTERVAL) {
            Ok(response) => return Ok(Some(with_request_id(&response, request_id))),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(Some(error_json(
                    request_id,
                    "server_unavailable",
                    "the server stopped before the picker resolved",
                )));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if should_stop_connection(stream, running)? {
                    let (respond_to, _) = std::sync::mpsc::channel();
                    let _ = api_tx.send(crate::api::ApiRequestMessage {
                        request: Request {
                            id: format!("{pick_id}:close"),
                            method: Method::UiPickClose(crate::api::schema::UiPickCloseParams {
                                pick_id,
                            }),
                        },
                        respond_to,
                        response_write_complete: None,
                    });
                    return Ok(None);
                }
            }
        }
    }
}

fn with_request_id(response: &str, request_id: String) -> String {
    match serde_json::from_str::<serde_json::Value>(response) {
        Ok(mut value) => {
            value["id"] = serde_json::Value::String(request_id);
            value.to_string()
        }
        Err(_) => response.to_owned(),
    }
}

fn error_json(id: String, code: &str, message: &str) -> String {
    serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    })
    .unwrap_or_else(|_| "{}".to_owned())
}
