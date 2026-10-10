//! Back/Forward steps as the shell performs them: what a step waits for, and
//! what it makes of the panes it sees meanwhile (`history.rs`). The client
//! loop's part, an endpoint handoff, is stood in for by the calls it makes:
//! it shows an endpoint, says when an activation completed, and says on
//! every turn whether it holds one. `endpoint/activation_tests.rs` runs a
//! step through the loop's own functions.

use super::fleet::{agent, fleet, machine};
use super::*;

/// A machine's snapshot with `focused` focused.
fn shown(boot_id: &str, pane_ids: &[&str], focused: &str, revision: u64) -> ClientShellSnapshot {
    let mut snapshot = machine(
        boot_id,
        pane_ids
            .iter()
            .map(|pane_id| agent(pane_id, AgentStatus::Idle, 1))
            .collect(),
    );
    snapshot.revision = revision;
    snapshot.focused_pane_id = Some(focused.into());
    for pane in &mut snapshot.panes {
        pane.focused = pane.pane_id == focused;
    }
    for agent in &mut snapshot.agents {
        agent.focused = agent.pane_id == focused;
    }
    snapshot
}

fn local(focused: &str, revision: u64) -> Box<ClientShellSnapshot> {
    Box::new(shown(
        "boot-1",
        &["p1", "p2", "p3", "p9"],
        focused,
        revision,
    ))
}

fn build(focused: &str, revision: u64) -> Box<ClientShellSnapshot> {
    Box::new(shown("build-boot", &["r5", "r9"], focused, revision))
}

/// Presses the Back or the Forward key.
pub(in crate::client) fn step(state: &mut ClientShellState, forward: bool) -> ClientShellInput {
    let action = if forward {
        crate::input::KeybindAction::HistoryForward
    } else {
        crate::input::KeybindAction::HistoryBack
    };
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

/// The focus request a step sent to the presented endpoint, as
/// `(request id, pane id)`.
fn focus_request(actions: &[ClientShellAction]) -> (String, String) {
    let [ClientShellAction::Endpoint { request, .. }] = actions else {
        panic!("expected one focus request: {actions:?}");
    };
    let crate::api::schema::Method::PaneFocus(target) = &request.method else {
        panic!("expected a pane focus request: {request:?}");
    };
    (request.id.clone(), target.pane_id.clone())
}

/// The activation a step asked the client loop for, as `(endpoint, pane id)`.
fn activation(input: &ClientShellInput) -> (ClientEndpointId, String) {
    match input.actions.as_slice() {
        [ClientShellAction::ActivateEndpoint {
            endpoint_id,
            target: Some(ClientEndpointFocusTarget::Pane(pane_id)),
        }] => (endpoint_id.clone(), pane_id.clone()),
        actions => panic!("expected one activation: {actions:?}"),
    }
}

fn answered(state: &mut ClientShellState, boot_id: &str, request_id: &str) {
    state.handle_endpoint_result(
        boot_id,
        request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
}

/// The client loop shows an endpoint. It does so rounds before a handoff
/// there completes, and for a handoff with a target only once the endpoint's
/// snapshot shows that target focused. It also does so when it reverses a
/// handoff, for the endpoint the handoff left.
fn show(
    state: &mut ClientShellState,
    endpoint_id: &ClientEndpointId,
    snapshot: Box<ClientShellSnapshot>,
) {
    state.set_endpoint_snapshot(endpoint_id, snapshot);
    assert!(state.activate_endpoint_projection(endpoint_id));
}

/// A handoff to an endpoint, with the pane its snapshot shows focused as the
/// target, runs to its end with no selection made meanwhile: the client loop
/// shows the endpoint, says the activation completed, and holds none on its
/// next turn.
fn present(
    state: &mut ClientShellState,
    endpoint_id: &ClientEndpointId,
    snapshot: Box<ClientShellSnapshot>,
) {
    let focus = snapshot
        .focused_pane_id
        .clone()
        .map(ClientEndpointFocusTarget::Pane);
    show(state, endpoint_id, snapshot);
    state.receive_endpoint_activated(endpoint_id, focus.as_ref());
    state.receive_endpoint_activations_held(false);
}

/// The client loop, holding no handoff, routes an activation of the endpoint
/// it presents back as a focus request, and holds none on its next turn.
fn routed_back(state: &mut ClientShellState, pane_id: String) -> (String, String) {
    let actions = state.focus_endpoint_target(ClientEndpointFocusTarget::Pane(pane_id));
    state.receive_endpoint_activations_held(false);
    focus_request(&actions)
}

/// A step to a Local pane while Local is presented among several endpoints.
/// The shell asks the client loop, which routes the focus back as a request.
fn local_step(state: &mut ClientShellState, forward: bool) -> (String, String) {
    let (endpoint_id, pane_id) = activation(&step(state, forward));
    assert_eq!(endpoint_id, ClientEndpointId::Local);
    routed_back(state, pane_id)
}

/// A client attached to Local and Build that visited Local `p1`, Build `r5`,
/// Local `p1` and Local `p2`, and presents the last.
fn visited() -> (ClientShellState, ClientEndpointId) {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));
    present(&mut state, &ClientEndpointId::Local, local("p1", 2));
    state.set_snapshot(local("p2", 3));
    (state, remote)
}

#[test]
fn a_pane_focused_from_outside_before_a_step_is_answered_keeps_the_forward_list() {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(local("p1", 1));
    state.set_snapshot(local("p2", 2));
    state.set_snapshot(local("p3", 3));

    let (request_id, pane_id) = focus_request(&step(&mut state, false).actions);
    assert_eq!(pane_id, "p2");
    // A script focuses `p9` before the endpoint carries out the step.
    state.set_snapshot(local("p9", 4));
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p2", 5));

    assert_eq!(focus_request(&step(&mut state, true).actions).1, "p3");
}

#[test]
fn a_pane_focused_after_a_step_was_carried_out_is_a_visit() {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(local("p1", 1));
    state.set_snapshot(local("p2", 2));
    state.set_snapshot(local("p3", 3));

    // The endpoint carried out the step, and the user's next focus request
    // behind it, before it sent a snapshot: `p2` never shows.
    let (request_id, _) = focus_request(&step(&mut state, false).actions);
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p9", 4));

    // `p9` followed `p2`, so nothing lies ahead and `p2` is one step back.
    assert!(step(&mut state, true).actions.is_empty());
    assert_eq!(focus_request(&step(&mut state, false).actions).1, "p2");
}

#[test]
fn a_second_step_is_not_confirmed_by_the_pane_the_first_one_is_leaving() {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(local("p1", 1));
    state.set_snapshot(local("p2", 2));
    state.set_snapshot(local("p1", 3));

    // Two quick steps back: to `p2`, then to the first `p1`. The second
    // request waits behind the first, and a snapshot that still shows the
    // last `p1` arrives meanwhile. It equals the second step's target.
    let (first, pane_id) = focus_request(&step(&mut state, false).actions);
    assert_eq!(pane_id, "p2");
    let (second, pane_id) = focus_request(&step(&mut state, false).actions);
    assert_eq!(pane_id, "p1");
    state.set_snapshot(local("p1", 4));

    // Each step is carried out in turn. The endpoint sends the snapshot of
    // the second one ahead of its answer.
    answered(&mut state, "boot-1", &first);
    state.set_snapshot(local("p2", 5));
    state.set_snapshot(local("p1", 6));
    answered(&mut state, "boot-1", &second);

    // The cursor is on the first `p1` with both later visits ahead, and the
    // step is over: a pane focused now is a visit.
    assert_eq!(focus_request(&step(&mut state, true).actions).1, "p2");
    let (request_id, pane_id) = focus_request(&step(&mut state, true).actions);
    assert_eq!(pane_id, "p1");
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 7));
    state.set_snapshot(local("p9", 8));
    assert!(step(&mut state, true).actions.is_empty());
    assert_eq!(focus_request(&step(&mut state, false).actions).1, "p1");
}

#[test]
fn a_step_the_endpoint_cannot_take_is_refused_on_the_spot() {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(local("p1", 1));
    state.set_snapshot(local("p2", 2));
    state.set_snapshot(local("p3", 3));

    state.set_endpoint_methods(Some(Vec::new()));
    assert!(step(&mut state, false).actions.is_empty());
    state.set_endpoint_methods(None);

    // The cursor is back on `p3`: the next step goes to `p2` again, and a
    // pane focused meanwhile is a visit.
    assert_eq!(focus_request(&step(&mut state, false).actions).1, "p2");
}

#[test]
fn a_step_to_another_machine_arrives_whatever_either_machine_had_focused() {
    let (mut state, remote) = visited();

    let (request_id, pane_id) = local_step(&mut state, false);
    assert_eq!(pane_id, "p1");
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 4));

    // Back to Build `r5`. While the client was away, Build's focus moved to
    // `r9`, and during the handoff a script focuses `p9` on Local.
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote.clone(), "r5".to_owned())
    );
    state.set_endpoint_snapshot(&remote, build("r9", 2));
    state.set_snapshot(local("p9", 5));
    present(&mut state, &remote, build("r5", 3));
    assert!(state.history_handoff.is_none());

    // Every later visit is still ahead.
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p1", 6));
    let (_, pane_id) = local_step(&mut state, true);
    assert_eq!(pane_id, "p2");
}

#[test]
fn a_step_carried_out_by_an_activation_of_the_machine_still_shown_has_arrived() {
    let (mut state, remote) = visited();

    // Back to Local `p1` from Local `p2`. The client loop does not route
    // the focus back as a request this time: it activates Local anew, as it
    // does to abandon a handoff under way, and presents it with `p1`
    // focused.
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p1", 4));
    assert!(state.history_handoff.is_none());

    // A request answered later finds no step to give up on: `p2` is still
    // one step ahead, and Build one step back.
    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
            pane_id: "p1".into(),
        }),
        &mut outcome,
    );
    answered(&mut state, "boot-1", &focus_request(&outcome.actions).0);
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p2".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p2", 5));
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p1", 6));
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote, "r5".to_owned())
    );
}

#[test]
fn a_step_whose_handoff_fails_returns_to_where_it_began() {
    let (mut state, remote) = visited();
    let (request_id, _) = local_step(&mut state, false);
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 4));

    assert_eq!(activation(&step(&mut state, false)).0, remote);
    // The handoff is rolled back: the client loop shows Local again, where
    // the focus moved meanwhile. That alone ends nothing, because the loop
    // may go on to another activation.
    show(&mut state, &ClientEndpointId::Local, local("p9", 5));
    assert!(state.history_handoff.is_some());
    // The loop gave the activation up and holds none. No request follows.
    state.receive_endpoint_unavailable("Build did not answer".into());
    state.receive_endpoint_activations_held(false);

    // The cursor is back on Local `p1`, and `p9`, which went unrecorded
    // while the step was awaited, follows it: one step back is `p1`, and
    // the next is Build again.
    assert!(state.history_handoff.is_none());
    assert!(step(&mut state, true).actions.is_empty());
    let (request_id, pane_id) = local_step(&mut state, false);
    assert_eq!(pane_id, "p1");
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 6));
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote, "r5".to_owned())
    );
}

#[test]
fn a_failed_activation_of_another_machine_leaves_the_step_under_way() {
    let (mut state, remote) = visited();
    let (request_id, _) = local_step(&mut state, false);
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 4));

    // Back to Build `r5`. A selection of Local that the client loop could
    // not begin is not the step's activation: the loop still holds that one.
    assert_eq!(activation(&step(&mut state, false)).0, remote);
    state.receive_endpoint_unavailable("Local: not ready".into());
    state.receive_endpoint_activations_held(true);
    assert!(state.history_handoff.is_some());
    state.set_snapshot(local("p9", 5));

    // The step arrives with everything after it still ahead.
    present(&mut state, &remote, build("r5", 2));
    assert!(state.history_handoff.is_none());
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
}

#[test]
fn a_step_replaced_by_another_selection_returns_to_where_it_began() {
    let (mut state, remote) = visited();
    let (request_id, _) = local_step(&mut state, false);
    answered(&mut state, "boot-1", &request_id);
    state.set_snapshot(local("p1", 4));

    // Back to Build `r5`, but the handoff ends on `r9`: the user picked it
    // on the way.
    assert_eq!(activation(&step(&mut state, false)).0, remote);
    present(&mut state, &remote, build("r9", 2));

    // `r9` follows Local `p1`, where the step began, and Build `r5` is
    // still one step further back.
    assert!(step(&mut state, true).actions.is_empty());
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p1", 5));
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote, "r5".to_owned())
    );
}

#[test]
fn a_routed_steps_refusal_rewinds_like_a_direct_ones() {
    let (mut state, remote) = visited();

    // Local `p1` closed. The request the client loop routes back keeps the
    // step's kind, so its refusal ends the step and forgets the pane.
    let (request_id, pane_id) = local_step(&mut state, false);
    assert_eq!(pane_id, "p1");
    state.handle_endpoint_result(
        "boot-1",
        &request_id,
        Err(ClientShellEndpointError {
            code: Some("pane_not_found".into()),
            message: "pane p1 not found".into(),
        }),
    );

    // Both visits to `p1` are gone: one step back from `p2` is Build, and
    // one step forward from there is `p2` again.
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote.clone(), "r5".to_owned())
    );
    present(&mut state, &remote, build("r5", 2));
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p2".to_owned())
    );
}

#[test]
fn a_second_step_during_a_handoff_from_a_remote_machine_replaces_it_through_the_client_loop() {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    let build = |focused: &str, revision: u64| {
        Box::new(shown("build-boot", &["r5", "r7", "r9"], focused, revision))
    };
    // Visited: Local `p1`, Build `r5`, Local `p1`, Build `r9`, Build `r7`.
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));
    present(&mut state, &ClientEndpointId::Local, local("p1", 2));
    present(&mut state, &remote, build("r9", 2));
    state.set_snapshot(build("r7", 3));

    // One step back, carried out: Build `r9` is presented, `r7` lies ahead.
    let (request_id, pane_id) = focus_request(&step(&mut state, false).actions);
    assert_eq!(pane_id, "r9");
    answered(&mut state, "build-boot", &request_id);
    state.set_snapshot(build("r9", 4));

    // Back to Local `p1`: a handoff. Back again before it ends: Build `r5`,
    // on the machine still presented. The client loop freezes input for the
    // handoff and would cancel a focus request, so this step is an
    // activation too, which the loop puts in the first one's place.
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote.clone(), "r5".to_owned())
    );
    // The loop reverses the first handoff, which shows Build as it was, and
    // then carries out the second step.
    show(&mut state, &remote, build("r9", 5));
    assert!(state.history_handoff.is_some());
    present(&mut state, &remote, build("r5", 6));
    assert!(state.history_handoff.is_none());

    // The cursor is on Build `r5` with every later visit still ahead.
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    present(&mut state, &ClientEndpointId::Local, local("p1", 3));
    assert_eq!(
        activation(&step(&mut state, true)),
        (remote.clone(), "r9".to_owned())
    );
    present(&mut state, &remote, build("r9", 7));
    assert_eq!(focus_request(&step(&mut state, true).actions).1, "r7");
}

#[test]
fn a_step_after_a_handoff_the_client_loop_never_began_is_routed_back_as_its_request() {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    let build = |focused: &str, revision: u64| {
        Box::new(shown("build-boot", &["r5", "r7", "r9"], focused, revision))
    };
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));
    state.set_snapshot(build("r9", 2));
    state.set_snapshot(build("r7", 3));

    // Two steps back, carried out: Build `r9`, then Build `r5`. A third
    // leads to Local `p1` and leaves as a handoff, which the client loop
    // does not always begin: it puts a selection of Local off while Local
    // reconnects, and holds it. Forward is then an activation of the
    // machine presented, which the loop takes in place of the selection it
    // put off and, with no handoff under way, routes back as the step's own
    // focus request.
    let (request_id, _) = focus_request(&step(&mut state, false).actions);
    answered(&mut state, "build-boot", &request_id);
    state.set_snapshot(build("r9", 4));
    let (request_id, _) = focus_request(&step(&mut state, false).actions);
    answered(&mut state, "build-boot", &request_id);
    state.set_snapshot(build("r5", 5));
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    state.receive_endpoint_activations_held(true);
    let (endpoint_id, pane_id) = activation(&step(&mut state, true));
    assert_eq!((&endpoint_id, pane_id.as_str()), (&remote, "r5"));
    let (request_id, pane_id) = routed_back(&mut state, pane_id);
    assert_eq!(pane_id, "r5");
    assert!(state.history_handoff.is_none());
    answered(&mut state, "build-boot", &request_id);

    // The step ended on Build `r5`, where the chain began: `r9` and `r7`
    // are still ahead.
    assert_eq!(focus_request(&step(&mut state, true).actions).1, "r9");
}

/// A client attached to Local and Build that visited Local `p1`, Build `r5`,
/// Local `p1` and Build `r5` again, and presents the last.
fn toggled() -> (ClientShellState, ClientEndpointId) {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));
    present(&mut state, &ClientEndpointId::Local, local("p1", 2));
    present(&mut state, &remote, build("r5", 2));
    (state, remote)
}

#[test]
fn a_machine_restored_ahead_of_a_second_steps_activation_is_not_that_steps_arrival() {
    let (mut state, remote) = toggled();

    // Back to Local `p1`, a handoff, and Back again before it ends, to the
    // first Build `r5`.
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote.clone(), "r5".to_owned())
    );
    // The loop reverses the first handoff, which shows Build as it was: with
    // `r5` focused, the pane the second step leads to. That step's own
    // activation has not begun, and a script focuses `r9` before it does.
    show(&mut state, &remote, build("r5", 3));
    assert!(state.history_handoff.is_some());
    state.set_snapshot(build("r9", 4));

    // The step's activation completes on `r5`. Every later visit is still
    // ahead.
    present(&mut state, &remote, build("r5", 5));
    assert!(state.history_handoff.is_none());
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
}

#[test]
fn a_second_step_whose_own_activation_fails_returns_to_where_the_chain_began() {
    let (mut state, remote) = toggled();

    assert_eq!(
        activation(&step(&mut state, false)).0,
        ClientEndpointId::Local
    );
    assert_eq!(activation(&step(&mut state, false)).0, remote);
    // The loop reverses the first handoff, showing Build with `r5` focused,
    // then gives up the second step's own activation and holds none.
    show(&mut state, &remote, build("r5", 3));
    state.receive_endpoint_unavailable("Build did not answer".into());
    state.receive_endpoint_activations_held(false);

    // The pane shown is the visit the chain began on, not the equal one two
    // steps back: nothing lies ahead, and one step back is Local `p1`.
    assert!(state.history_handoff.is_none());
    assert!(step(&mut state, true).actions.is_empty());
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
}

#[test]
fn a_second_step_pressed_once_the_first_handoff_shows_its_target_keeps_the_forward_list() {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    let build = |focused: &str, revision: u64| {
        Box::new(shown("build-boot", &["r5", "r7", "r9"], focused, revision))
    };
    // Visited: Local `p1`, Build `r5`, Local `p1`, Build `r9`, Build `r7`.
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));
    present(&mut state, &ClientEndpointId::Local, local("p1", 2));
    present(&mut state, &remote, build("r9", 2));
    state.set_snapshot(build("r7", 3));

    // One step back, carried out: Build `r9` is presented, `r7` lies ahead.
    let (request_id, pane_id) = focus_request(&step(&mut state, false).actions);
    assert_eq!(pane_id, "r9");
    answered(&mut state, "build-boot", &request_id);
    state.set_snapshot(build("r9", 4));

    // Back to Local `p1`: a handoff, which shows Local with the pane focused
    // two request rounds before it completes. Back again in between: Build
    // `r5`.
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    show(&mut state, &ClientEndpointId::Local, local("p1", 3));
    assert_eq!(
        activation(&step(&mut state, false)),
        (remote.clone(), "r5".to_owned())
    );
    // The loop reverses the handoff, which shows Build as it was, and then
    // carries out the second step.
    show(&mut state, &remote, build("r9", 5));
    present(&mut state, &remote, build("r5", 6));

    // The cursor is on Build `r5` with every later visit still ahead.
    assert_eq!(
        activation(&step(&mut state, true)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
}

#[test]
fn a_step_the_client_loop_put_off_is_not_ended_by_another_answered_request() {
    let (mut state, remotes) = fleet(&["Build"]);
    let remote = remotes[0].clone();
    state.set_snapshot(local("p1", 1));
    present(&mut state, &remote, build("r5", 1));

    // Back to Local `p1`: a handoff, which the loop puts off while Local
    // reconnects, and holds. Build answers another request meanwhile.
    assert_eq!(
        activation(&step(&mut state, false)),
        (ClientEndpointId::Local, "p1".to_owned())
    );
    state.receive_endpoint_activations_held(true);
    let mut outcome = ClientShellInput::default();
    state.push_endpoint_method(
        crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
            pane_id: "r5".into(),
        }),
        &mut outcome,
    );
    answered(&mut state, "build-boot", &focus_request(&outcome.actions).0);

    // Local is ready and the loop carries the step out: Build `r5` is still
    // one step ahead.
    present(&mut state, &ClientEndpointId::Local, local("p1", 2));
    assert_eq!(
        activation(&step(&mut state, true)),
        (remote, "r5".to_owned())
    );
}
