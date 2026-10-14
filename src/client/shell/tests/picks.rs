use super::*;
use crate::api::schema::{UiPickCreate, UiPickItem, UiPickOutcome};
use crate::client::shell::{ControlOrigin, ResolvedPick};
use crate::protocol::endpoint::{EndpointPickClose, EndpointPickOpen};

/// The connection Local's daemon opened the picker on.
fn local() -> ControlOrigin {
    ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    }
}

fn pick_open(pick_id: &str, create: bool) -> EndpointPickOpen {
    let item = |id: &str, label: &str| UiPickItem {
        id: id.into(),
        label: label.into(),
        detail: None,
        badge: None,
    };
    EndpointPickOpen {
        pick_id: pick_id.into(),
        title: "Space group".into(),
        items: vec![item("work", "Work"), item("home", "Home")],
        selected: Some("home".into()),
        create: create.then(|| UiPickCreate {
            label: "New group".into(),
        }),
    }
}

fn shell_with_pick(create: bool) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot()));
    state.set_pane_surface(surface());
    assert!(state.receive_pick_open(local(), pick_open("pick-1", create)));
    state
}

/// The one answer a client-loop turn would send for `pick-1`.
fn resolved(state: &mut ClientShellState) -> UiPickOutcome {
    let answers = state.settle_picks();
    let [ResolvedPick {
        origin,
        pick_id,
        outcome,
    }] = answers.as_slice()
    else {
        panic!("expected one answer: {answers:?}");
    };
    assert_eq!(origin, &local());
    assert_eq!(pick_id, "pick-1");
    assert!(state.settle_picks().is_empty(), "answered once");
    outcome.clone()
}

#[test]
fn enter_picks_the_preselected_item_and_typing_filters() {
    let mut state = shell_with_pick(false);
    let input = state.handle_input_bytes(b"\r");
    assert!(input.actions.is_empty(), "answers bypass the command lane");
    assert_eq!(
        resolved(&mut state),
        UiPickOutcome::Picked { id: "home".into() }
    );
    assert!(state.overlay.is_none());

    let mut state = shell_with_pick(false);
    state.handle_input_bytes(b"wo");
    state.handle_input_bytes(b"\r");
    assert_eq!(
        resolved(&mut state),
        UiPickOutcome::Picked { id: "work".into() }
    );
}

#[test]
fn created_text_too_long_for_one_command_cancels_the_picker_and_says_so() {
    let limit = crate::server::client_commands::MAX_ENDPOINT_COMMAND_BYTES;

    // Text that fits is sent as typed, however long.
    let mut state = shell_with_pick(true);
    assert!(state.insert_pick_text(&"x".repeat(limit / 2)));
    let input = state.handle_input_bytes(b"\r");
    assert!(input.repaint);
    assert!(matches!(
        resolved(&mut state),
        UiPickOutcome::Created { text } if text.len() == limit / 2
    ));
    assert!(state.visible_endpoint_notice.is_none());

    // Text the daemon would never read closes the picker all the same, so
    // the plugin hears `cancelled` instead of waiting, and the user is told.
    let mut state = shell_with_pick(true);
    assert!(state.insert_pick_text(&"x".repeat(limit)));
    let input = state.handle_input_bytes(b"\r");
    assert!(input.repaint);
    assert!(state.overlay.is_none());
    assert_eq!(resolved(&mut state), UiPickOutcome::Cancelled);
    let notice = state.visible_endpoint_notice.as_ref().expect("a notice");
    assert_eq!(notice.title, "Picker cancelled");

    // Escaping counts: the limit is on the request as it is written.
    let mut state = shell_with_pick(true);
    assert!(state.insert_pick_text(&"\"".repeat(limit / 2)));
    state.handle_input_bytes(b"\r");
    assert_eq!(resolved(&mut state), UiPickOutcome::Cancelled);
}

#[test]
fn unmatched_query_offers_create_only_when_allowed() {
    let mut state = shell_with_pick(true);
    state.handle_input_bytes(b"Side");
    state.handle_input_bytes(b"\r");
    assert_eq!(
        resolved(&mut state),
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
    assert!(state.settle_picks().is_empty(), "still open");
}

#[test]
fn escape_cancels_and_a_server_close_dismisses_silently() {
    let mut state = shell_with_pick(false);
    state.handle_input_bytes(b"\x1b");
    assert_eq!(resolved(&mut state), UiPickOutcome::Cancelled);

    let mut state = shell_with_pick(false);
    assert!(!state.receive_pick_close(
        &local(),
        EndpointPickClose {
            pick_id: "other".into()
        }
    ));
    assert!(state.receive_pick_close(
        &local(),
        EndpointPickClose {
            pick_id: "pick-1".into()
        }
    ));
    assert!(state.overlay.is_none());
    assert!(
        state.settle_picks().is_empty(),
        "the daemon already answered"
    );
}

#[test]
fn a_picker_removed_by_anything_else_is_answered_cancelled() {
    let mut state = shell_with_pick(false);
    // Any other overlay, or none, replaces the picker without a key reaching it.
    state.overlay = None;
    assert_eq!(resolved(&mut state), UiPickOutcome::Cancelled);

    // A second picker from the presented endpoint replaces the first.
    let mut state = shell_with_pick(false);
    assert!(state.receive_pick_open(local(), pick_open("pick-2", false)));
    let answers = state.settle_picks();
    assert_eq!(
        answers,
        [ResolvedPick {
            origin: local(),
            pick_id: "pick-1".into(),
            outcome: UiPickOutcome::Cancelled,
        }]
    );
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == "pick-2"
    ));
}

#[test]
fn a_picker_is_cancelled_when_the_presented_endpoints_connection_drops() {
    let mut state = shell_with_pick(false);

    state.mark_endpoint_disconnected(&ClientEndpointId::Local);

    assert!(state.overlay.is_none());
    // The answer names the connection that dropped, so the client loop
    // never sends it on the next one.
    assert_eq!(resolved(&mut state), UiPickOutcome::Cancelled);
}

#[test]
fn a_picker_from_an_endpoint_not_presented_is_cancelled_as_it_arrives() {
    let mut state = shell_with_pick(false);
    let elsewhere = ControlOrigin {
        endpoint_id: ClientEndpointId::Ssh(
            crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef")
                .expect("ssh profile id"),
        ),
        generation: 2,
    };
    assert!(!state.receive_pick_open(elsewhere.clone(), pick_open("pick-9", false)));
    assert_eq!(
        state.settle_picks(),
        [ResolvedPick {
            origin: elsewhere,
            pick_id: "pick-9".into(),
            outcome: UiPickOutcome::Cancelled,
        }]
    );
    // The presented endpoint's picker stays open and unanswered.
    assert!(matches!(
        &state.overlay,
        Some(ClientShellOverlay::Pick(pick)) if pick.pick_id == "pick-1"
    ));
    assert!(state.settle_picks().is_empty());
}

#[test]
fn clicking_a_row_picks_it() {
    let mut state = shell_with_pick(false);
    state.compose(106, 30).expect("picker frame");
    let (row, _) = *state
        .hits
        .worktree_rows
        .iter()
        .find(|(_, index)| *index == 0)
        .expect("a hit for the first row");
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: row.x + 2,
        row: row.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert_eq!(
        resolved(&mut state),
        UiPickOutcome::Picked { id: "work".into() }
    );
    assert!(state.overlay.is_none());
}
