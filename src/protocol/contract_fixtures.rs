//! The fixtures for the wire shapes this fork adds between the daemon and its
//! clients. Herdr GPUI's protocol crate keeps verbatim copies of these files,
//! so each test pins a fixture to the exact value the daemon decodes and to
//! its canonical encoding.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::endpoint::{
    pick_close_message, pick_open_message, EndpointPickClose, EndpointPickOpen, PICK_CLOSE_KIND,
    PICK_OPEN_KIND,
};
use super::ServerMessage;
use crate::api::schema::{
    Method, PaneTarget, Request, UiPickCreate, UiPickItem, UiPickOutcome, UiPickResolveParams,
};

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/",
            $name
        ))
    };
}

/// The fixture decodes to `expected`, and `expected` encodes to exactly the
/// fixture's JSON: no field more, none missing.
fn assert_canonical<T>(fixture: &str, expected: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json: serde_json::Value = serde_json::from_str(fixture).expect("fixture is JSON");
    let decoded: T = serde_json::from_value(json.clone()).expect("fixture decodes");
    assert_eq!(&decoded, expected);
    assert_eq!(serde_json::to_value(expected).expect("value encodes"), json);
}

fn pick_open() -> EndpointPickOpen {
    EndpointPickOpen {
        pick_id: "pick-7".into(),
        title: "Move pane to".into(),
        items: vec![
            UiPickItem {
                id: "t1".into(),
                label: "shell".into(),
                detail: None,
                badge: None,
            },
            UiPickItem {
                id: "t2".into(),
                label: "build".into(),
                detail: Some("3 panes".into()),
                badge: Some("2".into()),
            },
        ],
        selected: Some("t2".into()),
        create: Some(UiPickCreate {
            label: "New tab".into(),
        }),
    }
}

#[test]
fn pick_open_fixture_is_the_control_payload() {
    let expected = pick_open();
    assert_canonical(fixture!("endpoint-pick-open-v1.json"), &expected);

    let ServerMessage::EndpointControl { kind, data } = pick_open_message(&expected).unwrap()
    else {
        panic!("opening a picker is an endpoint control");
    };
    assert_eq!(kind, PICK_OPEN_KIND);
    assert_eq!(kind, "endpoint.pick.open.v1");
    assert_eq!(
        serde_json::from_str::<EndpointPickOpen>(&data).unwrap(),
        expected
    );
}

#[test]
fn pick_close_fixture_is_the_control_payload() {
    let expected = EndpointPickClose {
        pick_id: "pick-7".into(),
    };
    assert_canonical(fixture!("endpoint-pick-close-v1.json"), &expected);

    let ServerMessage::EndpointControl { kind, data } = pick_close_message("pick-7").unwrap()
    else {
        panic!("closing a picker is an endpoint control");
    };
    assert_eq!(kind, PICK_CLOSE_KIND);
    assert_eq!(kind, "endpoint.pick.close.v1");
    assert_eq!(
        serde_json::from_str::<EndpointPickClose>(&data).unwrap(),
        expected
    );
}

#[test]
fn pick_resolve_fixture_lists_every_outcome() {
    let expected = [
        UiPickOutcome::Picked { id: "t2".into() },
        UiPickOutcome::Created {
            text: "New tab name".into(),
        },
        UiPickOutcome::Cancelled,
    ]
    .map(|outcome| UiPickResolveParams {
        pick_id: "pick-7".into(),
        outcome,
    });
    assert_canonical(
        fixture!("endpoint-pick-resolve-v1.json"),
        &expected.to_vec(),
    );

    // The params are what the client sends on its command lane.
    let request = serde_json::to_value(Request {
        id: "client-shell:1".into(),
        method: Method::UiPickResolve(expected[0].clone()),
    })
    .unwrap();
    assert_eq!(request["method"], "ui.pick.resolve");
    assert_eq!(
        request["params"],
        serde_json::from_str::<serde_json::Value>(fixture!("endpoint-pick-resolve-v1.json"))
            .unwrap()[0]
    );
}

#[test]
fn agent_mark_unread_fixture_is_the_request() {
    let expected = Request {
        id: "client-shell:14".into(),
        method: Method::AgentMarkUnread(PaneTarget {
            pane_id: "w1:p1".into(),
        }),
    };
    assert_canonical(fixture!("endpoint-agent-mark-unread-v1.json"), &expected);
}
