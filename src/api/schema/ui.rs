//! Native UI that plugins drive through the socket API.

use serde::{Deserialize, Serialize};

/// One row of a native picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiPickItem {
    pub id: String,
    pub label: String,
    /// Secondary text shown after the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Short marker shown before the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
}

/// Offers a "create" row for query text that matches no item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiPickCreate {
    /// Row label prefix, such as "New group".
    pub label: String,
}

/// Opens a picker on a TUI client and waits until it resolves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiPickParams {
    pub title: String,
    pub items: Vec<UiPickItem>,
    /// Id of the item selected when the picker opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<UiPickCreate>,
    /// Client to show the picker on; defaults to the foreground client.
    /// Plugins pass the `client_id` from their invocation context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiPickOutcome {
    Picked { id: String },
    Created { text: String },
    Cancelled,
}

/// Sent by the client that shows a picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiPickResolveParams {
    pub pick_id: String,
    pub outcome: UiPickOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UiPickCloseParams {
    pub pick_id: String,
}
