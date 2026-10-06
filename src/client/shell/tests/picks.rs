use super::*;
use crate::api::schema::{Method, UiPickCreate, UiPickItem, UiPickOutcome};
use crate::protocol::endpoint::{EndpointPickClose, EndpointPickOpen};

fn shell_with_pick(create: bool) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    let item = |id: &str, label: &str| UiPickItem {
        id: id.into(),
        label: label.into(),
        detail: None,
        badge: None,
    };
    assert!(state.receive_pick_open(
        &ClientEndpointId::Local,
        EndpointPickOpen {
            pick_id: "pick-1".into(),
            title: "Space group".into(),
            items: vec![item("work", "Work"), item("home", "Home")],
            selected: Some("home".into()),
            create: create.then(|| UiPickCreate {
                label: "New group".into(),
            }),
        },
    ));
    state
}

fn resolved(input: &ClientShellInput) -> UiPickOutcome {
    let [ClientShellAction::Endpoint { request, .. }] = &input.actions[..] else {
        panic!("expected one resolve request");
    };
    let Method::UiPickResolve(params) = &request.method else {
        panic!("expected ui.pick.resolve");
    };
    assert_eq!(params.pick_id, "pick-1");
    params.outcome.clone()
}

#[test]
fn enter_picks_the_preselected_item_and_typing_filters() {
    let mut state = shell_with_pick(false);
    assert_eq!(
        resolved(&state.handle_input_bytes(b"\r")),
        UiPickOutcome::Picked { id: "home".into() }
    );
    assert!(state.overlay.is_none());

    let mut state = shell_with_pick(false);
    state.handle_input_bytes(b"wo");
    assert_eq!(
        resolved(&state.handle_input_bytes(b"\r")),
        UiPickOutcome::Picked { id: "work".into() }
    );
}

#[test]
fn unmatched_query_offers_create_only_when_allowed() {
    let mut state = shell_with_pick(true);
    state.handle_input_bytes(b"Side");
    assert_eq!(
        resolved(&state.handle_input_bytes(b"\r")),
        UiPickOutcome::Created {
            text: "Side".into()
        }
    );

    let mut state = shell_with_pick(true);
    state.handle_input_bytes(b"work");
    let Some(ClientShellOverlay::Pick(pick)) = &state.overlay else {
        panic!("picker closed");
    };
    assert_eq!(pick.rows(), vec![ClientPickRow::Item(0)]);

    let mut state = shell_with_pick(false);
    state.handle_input_bytes(b"Side");
    assert!(state.handle_input_bytes(b"\r").actions.is_empty());
    assert!(state.overlay.is_some());
}

#[test]
fn escape_cancels_and_a_server_close_dismisses_silently() {
    let mut state = shell_with_pick(false);
    assert_eq!(
        resolved(&state.handle_input_bytes(b"\x1b")),
        UiPickOutcome::Cancelled
    );

    let mut state = shell_with_pick(false);
    assert!(!state.receive_pick_close(
        &ClientEndpointId::Local,
        EndpointPickClose {
            pick_id: "other".into()
        }
    ));
    assert!(state.receive_pick_close(
        &ClientEndpointId::Local,
        EndpointPickClose {
            pick_id: "pick-1".into()
        }
    ));
    assert!(state.overlay.is_none());
}
