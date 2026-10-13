//! The trail, as Herdr GPUI tests `endpoint/history.rs`, then the keys that
//! step it in this client.

use super::*;
use crate::client::shell::tests::fleet::{fleet, profiles};
use crate::input::KeybindAction;
use crate::protocol::ClientShellPane;

/// A snapshot of `boot` with `panes` open and `focused` focused.
fn at(boot: &str, panes: &[&str], focused: Option<&str>) -> ClientShellSnapshot {
    let mut snapshot = crate::client::shell::tests::snapshot();
    let pane = snapshot.panes[0].clone();
    snapshot.panes = panes
        .iter()
        .map(|id| ClientShellPane {
            pane_id: (*id).to_owned(),
            ..pane.clone()
        })
        .collect();
    snapshot.boot_id = boot.into();
    snapshot.focused_pane_id = focused.map(str::to_owned);
    snapshot
}

const PANES: &[&str] = &["a", "b", "c", "d"];

/// A snapshot focusing `focused` while a navigation may still be in flight.
fn visit(history: &mut History, focused: &str) -> ClientShellSnapshot {
    let snapshot = at("boot", PANES, Some(focused));
    history.observe(Some(&snapshot), true);
    snapshot
}

/// Starts `step` as the shell does once its request is queued.
fn start(history: &mut History, step: Step, snapshot: &ClientShellSnapshot) -> Option<String> {
    let (index, pane) = history
        .peek(step, snapshot)
        .map(|(index, pane)| (index, pane.to_owned()))?;
    history.begin(index);
    Some(pane)
}

/// Goes `step`, then reports the focus arriving and the navigation settling.
fn go(history: &mut History, step: Step, snapshot: &ClientShellSnapshot) -> String {
    let pane = start(history, step, snapshot).unwrap();
    history.observe(Some(&at("boot", PANES, Some(&pane))), false);
    pane
}

#[test]
fn back_and_forward_walk_the_focus_trail() {
    let mut history = History::default();
    let mut now = visit(&mut history, "a");
    assert!(!history.can(Step::Back, &now));
    for pane in ["b", "c"] {
        now = visit(&mut history, pane);
    }
    assert!(!history.can(Step::Forward, &now));
    assert_eq!(go(&mut history, Step::Back, &now), "b");
    now = at("boot", PANES, Some("b"));
    assert_eq!(go(&mut history, Step::Back, &now), "a");
    now = at("boot", PANES, Some("a"));
    assert!(!history.can(Step::Back, &now));
    assert_eq!(go(&mut history, Step::Forward, &now), "b");
    now = at("boot", PANES, Some("b"));
    assert_eq!(go(&mut history, Step::Forward, &now), "c");
}

#[test]
fn going_somewhere_new_drops_the_entries_ahead() {
    let mut history = History::default();
    for pane in ["a", "b", "c"] {
        visit(&mut history, pane);
    }
    let now = at("boot", PANES, Some("c"));
    go(&mut history, Step::Back, &now);
    let now = visit(&mut history, "d");
    assert!(!history.can(Step::Forward, &now));
    assert_eq!(go(&mut history, Step::Back, &now), "b");
}

#[test]
fn a_second_step_before_the_first_lands_continues_from_it() {
    let mut history = History::default();
    for pane in ["a", "b", "c"] {
        visit(&mut history, pane);
    }
    let now = at("boot", PANES, Some("c"));
    assert_eq!(start(&mut history, Step::Back, &now).as_deref(), Some("b"));
    assert_eq!(start(&mut history, Step::Back, &now).as_deref(), Some("a"));
    // A snapshot that changes something else keeps the travel pending.
    visit(&mut history, "c");
    visit(&mut history, "a");
    let now = at("boot", PANES, Some("a"));
    assert_eq!(history.peek(Step::Forward, &now), Some((1, "b")));
}

#[test]
fn peeking_changes_nothing_until_the_request_is_queued() {
    let mut history = History::default();
    for pane in ["a", "b", "c"] {
        visit(&mut history, pane);
    }
    let now = at("boot", PANES, Some("c"));
    // The first press is queued; a second one the shell cannot send only
    // peeks, so the first still lands as a step back.
    assert_eq!(start(&mut history, Step::Back, &now).as_deref(), Some("b"));
    assert_eq!(history.peek(Step::Back, &now), Some((0, "a")));
    history.observe(Some(&at("boot", PANES, Some("b"))), false);
    assert_eq!(
        history.peek(Step::Forward, &at("boot", PANES, Some("b"))),
        Some((2, "c"))
    );
}

#[test]
fn a_travel_ends_with_its_navigation() {
    for settles in [
        // The daemon answered without focusing the pane, say it had closed.
        Some(at("boot", PANES, Some("c"))),
        // The connection dropped and the next one starts empty.
        None,
    ] {
        let mut history = History::default();
        for pane in ["a", "b", "c"] {
            visit(&mut history, pane);
        }
        let now = at("boot", PANES, Some("c"));
        start(&mut history, Step::Back, &now);
        history.observe(settles.as_ref(), false);
        // The same boot returns on the old focus: Back starts from there
        // again, rather than from the pane that was never reached.
        history.observe(Some(&now), false);
        assert_eq!(history.peek(Step::Back, &now), Some((1, "b")));
        assert!(!history.can(Step::Forward, &now));
    }
}

#[test]
fn other_focus_ends_a_travel() {
    let mut history = History::default();
    for pane in ["a", "b", "c"] {
        visit(&mut history, pane);
    }
    let now = at("boot", PANES, Some("c"));
    start(&mut history, Step::Back, &now);
    // Focus moving elsewhere first is a new place, not the travel landing.
    let now = visit(&mut history, "d");
    assert!(!history.can(Step::Forward, &now));
    assert_eq!(go(&mut history, Step::Back, &now), "c");
}

#[test]
fn closed_and_focused_panes_are_stepped_over() {
    let mut history = History::default();
    for pane in ["a", "b", "a", "c"] {
        visit(&mut history, pane);
    }
    let closed = at("boot", &["a", "c"], Some("c"));
    assert_eq!(history.peek(Step::Back, &closed), Some((2, "a")));
    // Only the focused pane is left behind it.
    let alone = at("boot", &["c"], Some("c"));
    assert!(!history.can(Step::Back, &alone));
}

#[test]
fn the_trail_belongs_to_one_boot_and_survives_a_dropped_connection() {
    let mut history = History::default();
    visit(&mut history, "a");
    let now = visit(&mut history, "b");
    history.observe(None, false);
    history.observe(Some(&at("boot", PANES, None)), false);
    assert!(history.can(Step::Back, &now));
    // A snapshot from another boot cannot use this boot's trail.
    assert!(!history.can(Step::Back, &at("reboot", PANES, Some("b"))));
    let rebooted = at("reboot", PANES, Some("b"));
    history.observe(Some(&rebooted), false);
    assert!(!history.can(Step::Back, &rebooted));
}

#[test]
fn the_trail_is_bounded() {
    let mut history = History::default();
    let panes: Vec<String> = (0..LIMIT + 10).map(|i| format!("p{i}")).collect();
    for pane in &panes {
        history.observe(Some(&at("boot", &[pane.as_str()], Some(pane))), false);
    }
    assert_eq!(history.panes.len(), LIMIT);
    assert_eq!(history.panes.front().map(String::as_str), Some("p10"));
    assert_eq!(history.cursor, LIMIT - 1);
}

/// One machine with `pane_1` to `pane_3`, `focused` focused, at `revision`.
fn host(boot: &str, focused: &str, revision: u64) -> Box<ClientShellSnapshot> {
    let mut snapshot = at(boot, &["pane_1", "pane_2", "pane_3"], Some(focused));
    snapshot.revision = revision;
    Box::new(snapshot)
}

/// The Local machine after visiting `pane_1`, `pane_2`, then `pane_3`.
fn walked() -> ClientShellState {
    let (mut state, _) = fleet(&[]);
    for (revision, pane) in ["pane_1", "pane_2", "pane_3"].iter().enumerate() {
        state.set_snapshot(host("boot-1", pane, revision as u64 + 1));
    }
    state
}

/// Presses a history key. Returns the request, as `(request id, pane id)`,
/// and the endpoint it went to, if the press sent one.
fn press(state: &mut ClientShellState, step: Step) -> Option<(ClientEndpointId, String, String)> {
    let action = match step {
        Step::Back => KeybindAction::HistoryBack,
        Step::Forward => KeybindAction::HistoryForward,
    };
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    match outcome.actions.as_slice() {
        [] => None,
        [ClientShellAction::Endpoint {
            endpoint_id,
            request,
            ..
        }] => {
            let crate::api::schema::Method::PaneFocus(target) = &request.method else {
                panic!("expected a pane focus request: {request:?}");
            };
            Some((
                endpoint_id.clone(),
                request.id.clone(),
                target.pane_id.clone(),
            ))
        }
        actions => panic!("expected at most one focus request: {actions:?}"),
    }
}

#[test]
fn the_keys_focus_the_presented_endpoints_trail_and_land_on_it() {
    let mut state = walked();
    assert_eq!(press(&mut state, Step::Forward), None);

    let (endpoint, request_id, pane) = press(&mut state, Step::Back).unwrap();
    assert_eq!(endpoint, ClientEndpointId::Local);
    assert_eq!(pane, "pane_2");
    // The focus shows before the daemon answers; it is the step landing,
    // so the entry ahead survives.
    state.set_snapshot(host("boot-1", "pane_2", 4));
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    state.set_snapshot(host("boot-1", "pane_2", 5));

    assert_eq!(press(&mut state, Step::Forward).unwrap().2, "pane_3");
}

#[test]
fn a_refused_step_leaves_the_user_where_they_are() {
    let mut state = walked();
    let (_, request_id, pane) = press(&mut state, Step::Back).unwrap();
    assert_eq!(pane, "pane_2");

    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("pane_not_found".into()),
            message: "pane pane_2 not found".into(),
        }),
    );
    state.set_snapshot(host("boot-1", "pane_3", 4));

    // Back starts from pane_3 again, and nothing lies ahead of it.
    assert_eq!(press(&mut state, Step::Forward), None);
    assert_eq!(press(&mut state, Step::Back).unwrap().2, "pane_2");
}

#[test]
fn a_step_whose_request_is_gone_does_not_wait_for_a_snapshot_to_end() {
    for withdrawn in [false, true] {
        let mut state = walked();
        let (_, request_id, pane) = press(&mut state, Step::Back).unwrap();
        assert_eq!(pane, "pane_2");

        if withdrawn {
            state.cancel_endpoint_request(&request_id);
        } else {
            state.handle_endpoint_result(
                "boot-1",
                &request_id,
                Err(ClientShellEndpointError {
                    code: Some("pane_not_found".into()),
                    message: "pane pane_2 not found".into(),
                }),
            );
        }

        // No snapshot follows: pane_3 is still where the user is, so Back
        // starts from it rather than from the step that never landed.
        assert_eq!(
            press(&mut state, Step::Back).unwrap().2,
            "pane_2",
            "withdrawn: {withdrawn}"
        );
    }
}

#[test]
fn each_endpoint_keeps_its_own_trail_and_back_never_crosses_to_another() {
    let (mut state, remotes) = fleet(&["Build"]);
    let build = remotes[0].clone();
    state.set_snapshot(host("local-boot", "pane_1", 1));
    state.set_snapshot(host("local-boot", "pane_2", 2));
    // Build's focus moves while Local is presented: that is Build's trail.
    state.set_endpoint_snapshot(&build, host("build-boot", "pane_3", 1));
    state.set_endpoint_snapshot(&build, host("build-boot", "pane_1", 2));

    assert!(state.activate_endpoint_projection(&build));
    let (endpoint, _, pane) = press(&mut state, Step::Back).unwrap();
    assert_eq!(endpoint, build);
    assert_eq!(pane, "pane_3");

    // Build's first pane has nothing behind it, though Local's trail does.
    let (mut state, remotes) = fleet(&["Build"]);
    let build = remotes[0].clone();
    state.set_snapshot(host("local-boot", "pane_1", 1));
    state.set_snapshot(host("local-boot", "pane_2", 2));
    state.set_endpoint_snapshot(&build, host("build-boot", "pane_1", 1));
    assert!(state.activate_endpoint_projection(&build));
    assert_eq!(press(&mut state, Step::Back), None);
}

#[test]
fn a_removed_profile_loses_its_trail_and_a_reconnecting_one_keeps_it() {
    let (mut state, remotes) = fleet(&["Build", "Deploy"]);
    let (build, deploy) = (remotes[0].clone(), remotes[1].clone());
    let profiles = profiles(&["Build", "Deploy"]);
    state.set_snapshot(host("local-boot", "pane_1", 1));
    for remote in &remotes {
        state.set_endpoint_snapshot(remote, host("remote-boot", "pane_1", 1));
        state.set_endpoint_snapshot(remote, host("remote-boot", "pane_2", 2));
    }

    // Build drops and comes back to the same daemon while the catalog is
    // read again: its trail is kept.
    state.mark_endpoint_disconnected(&build);
    state.set_endpoint_catalog(&profiles);
    state.set_endpoint_status(&build, ClientEndpointStatus::Online);
    state.set_endpoint_snapshot(&build, host("remote-boot", "pane_2", 3));
    assert!(state.activate_endpoint_projection(&build));
    assert_eq!(press(&mut state, Step::Back).unwrap().2, "pane_1");

    // Deploy leaves the catalog: its trail goes, Build's and Local's stay.
    state.set_endpoint_catalog(&profiles[..1]);
    assert!(!state.fork.history.contains_key(&deploy));
    assert!(state.fork.history.contains_key(&build));
    assert!(state.fork.history.contains_key(&ClientEndpointId::Local));
}
