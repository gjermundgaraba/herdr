//! The fork's client contract controls: `ui.pick` pickers and `client.jump`
//! requests the server forwards to a client. Herdr GPUI copies them in
//! `crates/herdr-protocol`.

use serde::{Deserialize, Serialize};

use super::ServerMessage;

/// Server to client: show a `ui.pick` picker; the client answers with the
/// `ui.pick.resolve` method on the same connection.
pub const PICK_OPEN_KIND: &str = "endpoint.pick.open.v1";
/// Server to client: the picker closed without this client's answer.
pub const PICK_CLOSE_KIND: &str = "endpoint.pick.close.v1";
/// Server to client: show a pane; the client answers with the
/// `client.jump.resolve` method on the same connection.
pub const CLIENT_JUMP_KIND: &str = "endpoint.client-jump.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointPickOpen {
    pub pick_id: String,
    pub title: String,
    pub items: Vec<crate::api::schema::UiPickItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<crate::api::schema::UiPickCreate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointPickClose {
    pub pick_id: String,
}

/// A `client.jump` forwarded to the client that should perform it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointClientJump {
    pub jump_id: String,
    /// The endpoint in the client's own ids, as its report lists them.
    pub endpoint_id: String,
    pub boot_id: String,
    pub pane_id: String,
}

pub fn client_jump_message(jump: &EndpointClientJump) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: CLIENT_JUMP_KIND.into(),
        data: serde_json::to_string(jump)?,
    })
}

pub fn pick_open_message(open: &EndpointPickOpen) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: PICK_OPEN_KIND.into(),
        data: serde_json::to_string(open)?,
    })
}

pub fn pick_close_message(pick_id: &str) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: PICK_CLOSE_KIND.into(),
        data: serde_json::to_string(&EndpointPickClose {
            pick_id: pick_id.to_owned(),
        })?,
    })
}
