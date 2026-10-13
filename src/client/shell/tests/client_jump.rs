//! `endpoint.client-jump.v1` as the shell performs it: where a jump leads,
//! which navigation it starts, and how what the client presents answers it
//! (`client_jump.rs`).

use std::time::{Duration, Instant};

use super::fleet::{agent, fleet, machine};
use super::*;
use crate::api::schema::ClientJumpOutcome;
use crate::client::endpoint::ClientEndpointStatus;
use crate::client::shell::{ControlOrigin, ResolvedClientJump, NAVIGATION_SETTLE_DEADLINE};
use crate::protocol::endpoint::EndpointClientJump;

fn local_machine() -> ClientShellSnapshot {
    machine(
        "boot-1",
        vec![
            agent("pane_1", AgentStatus::Idle, 1),
            agent("pane_2", AgentStatus::Done, 2),
        ],
    )
}

/// A client attached to Local alone.
fn single() -> ClientShellState {
    let (mut state, _) = fleet(&[]);
    state.set_snapshot(Box::new(local_machine()));
    state
}

/// A client attached to Local and Build, presenting Local.
fn federated() -> (ClientShellState, ClientEndpointId) {
    let (mut state, remotes) = fleet(&["Build"]);
    state.set_snapshot(Box::new(local_machine()));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![
                agent("pane_1", AgentStatus::Idle, 3),
                agent("pane_7", AgentStatus::Blocked, 4),
            ],
        )),
    );
    (state, remotes[0].clone())
}

/// The connection the jump arrived on: Local's daemon forwarded it.
fn origin() -> ControlOrigin {
    ControlOrigin {
        endpoint_id: ClientEndpointId::Local,
        generation: 1,
    }
}

fn jump(endpoint_id: &str, boot_id: &str, pane_id: &str) -> EndpointClientJump {
    EndpointClientJump {
        jump_id: "jump-1".into(),
        endpoint_id: endpoint_id.into(),
        boot_id: boot_id.into(),
        pane_id: pane_id.into(),
    }
}

fn answer(outcome: ClientJumpOutcome) -> Vec<ResolvedClientJump> {
    vec![ResolvedClientJump {
        origin: origin(),
        jump_id: "jump-1".into(),
        outcome,
    }]
}

/// Receives a jump that must be answered without navigating.
fn refused(state: &mut ClientShellState, jump: EndpointClientJump) -> ClientJumpOutcome {
    let now = Instant::now();
    let input = state.receive_client_jump(origin(), jump, now);
    assert!(input.actions.is_empty(), "{:?}", input.actions);
    let mut resolved = state.settle_client_jumps(now, false);
    assert_eq!(resolved.len(), 1);
    assert!(
        state.settle_client_jumps(now, false).is_empty(),
        "answered once"
    );
    resolved.remove(0).outcome
}

/// The focus request a same-endpoint jump sent, as `(boot id, request id)`.
fn focus_request(
    input: &ClientShellInput,
    endpoint: &ClientEndpointId,
    pane: &str,
) -> (String, String) {
    match input.actions.as_slice() {
        [ClientShellAction::Endpoint {
            endpoint_id,
            boot_id,
            request,
            ..
        }] => {
            assert_eq!(endpoint_id, endpoint);
            assert!(
                matches!(&request.method,
                    crate::api::schema::Method::PaneFocus(target) if target.pane_id == pane),
                "{request:?}"
            );
            (boot_id.clone(), request.id.clone())
        }
        actions => panic!("expected one focus request: {actions:?}"),
    }
}

fn assert_activates(input: &ClientShellInput, endpoint: &ClientEndpointId, pane: &str) {
    assert!(
        matches!(input.actions.as_slice(),
            [ClientShellAction::ActivateEndpoint {
                endpoint_id,
                target: Some(ClientEndpointFocusTarget::Pane(target)),
            }] if endpoint_id == endpoint && target == pane),
        "{:?}",
        input.actions
    );
}

/// The next snapshot of the presented machine, with `pane_id` focused.
fn focus(state: &mut ClientShellState, pane_id: &str) {
    let mut next = state.snapshot.as_deref().expect("snapshot").clone();
    next.revision += 1;
    next.focused_pane_id = Some(pane_id.into());
    for pane in &mut next.panes {
        pane.focused = pane.pane_id == pane_id;
    }
    for agent in &mut next.agents {
        agent.focused = agent.pane_id == pane_id;
    }
    state.set_snapshot(Box::new(next));
}

#[test]
fn a_jump_on_the_presented_endpoint_focuses_the_pane_and_answers_once_it_shows() {
    let mut state = single();
    let start = Instant::now();

    let input = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);

    focus_request(&input, &ClientEndpointId::Local, "pane_2");
    assert!(state.settle_client_jumps(start, false).is_empty());

    // The endpoint accepted the request; the answer still waits for the
    // snapshot that shows the pane focused.
    let (boot_id, request_id) = focus_request(&input, &ClientEndpointId::Local, "pane_2");
    state.handle_endpoint_result(
        &boot_id,
        &request_id,
        Ok(crate::api::schema::ResponseResult::Ok {}),
    );
    assert!(state.settle_client_jumps(start, false).is_empty());

    focus(&mut state, "pane_2");
    assert_eq!(
        state.settle_client_jumps(start + Duration::from_millis(40), false),
        answer(ClientJumpOutcome::Focused)
    );
    assert!(
        state.settle_client_jumps(start, false).is_empty(),
        "answered once"
    );
}

#[test]
fn a_jump_to_the_pane_already_presented_is_answered_at_once() {
    let mut state = single();
    let now = Instant::now();

    let input = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_1"), now);

    assert!(input.actions.is_empty());
    assert_eq!(
        state.settle_client_jumps(now, false),
        answer(ClientJumpOutcome::Focused)
    );
}

#[test]
fn a_jump_to_another_endpoint_activates_it_with_the_pane_as_target() {
    let (mut state, remote) = federated();
    let start = Instant::now();

    // Both machines have a `pane_1`; the jump names Build's `pane_7`.
    let input = state.receive_client_jump(
        origin(),
        jump(&remote.storage_key(), "build-boot", "pane_7"),
        start,
    );

    assert_activates(&input, &remote, "pane_7");
    assert!(state.settle_client_jumps(start, false).is_empty());

    // Presenting Build is not enough: its focus is still elsewhere.
    assert!(state.activate_endpoint_projection(&remote));
    assert!(state.settle_client_jumps(start, false).is_empty());

    // Presenting Build with the pane focused is the answer.
    focus(&mut state, "pane_7");
    assert_eq!(
        state.settle_client_jumps(start + Duration::from_secs(1), false),
        answer(ClientJumpOutcome::Focused)
    );
}

#[test]
fn a_jump_back_to_local_from_a_federated_client_goes_through_activation() {
    let (mut state, remote) = federated();
    assert!(state.activate_endpoint_projection(&remote));
    let start = Instant::now();

    let input = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);

    assert_activates(&input, &ClientEndpointId::Local, "pane_2");
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    assert!(state.settle_client_jumps(start, false).is_empty());
    focus(&mut state, "pane_2");
    assert_eq!(
        state.settle_client_jumps(start, false),
        answer(ClientJumpOutcome::Focused)
    );
}

#[test]
fn a_pane_id_read_under_another_boot_is_stale() {
    let (mut state, remote) = federated();

    assert_eq!(
        refused(&mut state, jump("local", "boot-0", "pane_2")),
        ClientJumpOutcome::StaleBoot
    );
    assert_eq!(
        refused(&mut state, jump(&remote.storage_key(), "boot-1", "pane_7")),
        ClientJumpOutcome::StaleBoot,
        "Local's boot does not vouch for Build's panes"
    );
    // A restarted machine is stale before it is anything else.
    state.set_endpoint_status(&remote, ClientEndpointStatus::Reconnecting);
    assert_eq!(
        refused(
            &mut state,
            jump(&remote.storage_key(), "old-boot", "pane_7")
        ),
        ClientJumpOutcome::StaleBoot
    );
}

#[test]
fn an_unknown_pane_or_endpoint_is_not_found() {
    let (mut state, remote) = federated();

    assert_eq!(
        refused(&mut state, jump("local", "boot-1", "pane_7")),
        ClientJumpOutcome::NotFound,
        "pane_7 is Build's"
    );
    assert_eq!(
        refused(
            &mut state,
            jump(&remote.storage_key(), "build-boot", "pane_2")
        ),
        ClientJumpOutcome::NotFound
    );
    assert_eq!(
        refused(
            &mut state,
            jump("ssh:ffffffffffffffffffffffffffffffff", "boot-1", "pane_1")
        ),
        ClientJumpOutcome::NotFound,
        "no such machine in this client"
    );
    assert_eq!(
        refused(&mut state, jump("nostr:relay", "boot-1", "pane_1")),
        ClientJumpOutcome::NotFound
    );
}

#[test]
fn an_endpoint_that_is_not_online_is_unavailable() {
    let (mut state, remotes) = fleet(&["Build", "Lab"]);
    state.set_snapshot(Box::new(local_machine()));
    state.set_endpoint_snapshot(
        &remotes[0],
        Box::new(machine(
            "build-boot",
            vec![agent("pane_7", AgentStatus::Blocked, 4)],
        )),
    );

    for status in [
        ClientEndpointStatus::Reconnecting,
        ClientEndpointStatus::Attention,
        ClientEndpointStatus::Disabled,
    ] {
        state.set_endpoint_status(&remotes[0], status);
        assert_eq!(
            refused(
                &mut state,
                jump(&remotes[0].storage_key(), "build-boot", "pane_7")
            ),
            ClientJumpOutcome::Unavailable,
            "{status:?}"
        );
    }
    // Lab is online but has never sent a snapshot.
    assert_eq!(
        refused(
            &mut state,
            jump(&remotes[1].storage_key(), "lab-boot", "pane_1")
        ),
        ClientJumpOutcome::Unavailable
    );
    assert!(
        state.visible_endpoint_notice.is_none(),
        "a tool asked, not the user"
    );
}

#[test]
fn a_jump_that_never_shows_its_pane_times_out_before_the_daemon_does() {
    let (mut state, remote) = federated();
    let start = Instant::now();
    let input = state.receive_client_jump(
        origin(),
        jump(&remote.storage_key(), "build-boot", "pane_7"),
        start,
    );
    assert_activates(&input, &remote, "pane_7");

    let just_before = start + NAVIGATION_SETTLE_DEADLINE - Duration::from_millis(1);
    assert!(state.settle_client_jumps(just_before, false).is_empty());
    assert_eq!(
        state.settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE, false),
        answer(ClientJumpOutcome::Timeout)
    );
    assert!(state
        .settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE * 2, false)
        .is_empty());
    assert!(NAVIGATION_SETTLE_DEADLINE < Duration::from_secs(20));
}

#[test]
fn a_refused_or_withdrawn_navigation_is_not_followed_and_the_jump_times_out() {
    for withdrawn in [false, true] {
        let mut state = single();
        let start = Instant::now();
        let input = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);
        let (boot_id, request_id) = focus_request(&input, &ClientEndpointId::Local, "pane_2");

        if withdrawn {
            state.cancel_endpoint_request(&request_id);
        } else {
            state.handle_endpoint_result(
                &boot_id,
                &request_id,
                Err(ClientShellEndpointError {
                    code: Some("pane_focus_failed".into()),
                    message: "the pane cannot take focus".into(),
                }),
            );
        }

        // Only what the client presents answers the jump.
        assert!(state.settle_client_jumps(start, false).is_empty());
        assert_eq!(
            state.settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE, false),
            answer(ClientJumpOutcome::Timeout)
        );
    }
}

#[test]
fn a_jump_ends_when_its_endpoint_restarts_or_goes_away_meanwhile() {
    let (mut state, remote) = federated();
    let start = Instant::now();
    let key = remote.storage_key();

    state.receive_client_jump(origin(), jump(&key, "build-boot", "pane_7"), start);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Reconnecting);
    assert_eq!(
        state.settle_client_jumps(start, false),
        answer(ClientJumpOutcome::Unavailable)
    );

    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    state.receive_client_jump(origin(), jump(&key, "build-boot", "pane_7"), start);
    let mut restarted = machine("build-boot-2", vec![agent("pane_7", AgentStatus::Idle, 1)]);
    restarted.revision = 1;
    state.set_endpoint_snapshot(&remote, Box::new(restarted));
    assert_eq!(
        state.settle_client_jumps(start, false),
        answer(ClientJumpOutcome::StaleBoot)
    );

    state.receive_client_jump(origin(), jump(&key, "build-boot-2", "pane_7"), start);
    let mut closed = machine("build-boot-2", vec![agent("pane_1", AgentStatus::Idle, 1)]);
    closed.revision = 2;
    state.set_endpoint_snapshot(&remote, Box::new(closed));
    assert_eq!(
        state.settle_client_jumps(start, false),
        answer(ClientJumpOutcome::NotFound)
    );
}

#[test]
fn each_answer_keeps_the_connection_its_jump_arrived_on() {
    let (mut state, remote) = federated();
    let now = Instant::now();
    let from_build = ControlOrigin {
        endpoint_id: remote.clone(),
        generation: 9,
    };
    let mut second = jump("local", "boot-0", "pane_1");
    second.jump_id = "jump-2".into();

    // Two daemons forward a jump in the same turn; Build's names a Local pane.
    state.receive_client_jump(origin(), jump("local", "boot-1", "pane_1"), now);
    state.receive_client_jump(from_build.clone(), second, now);

    assert_eq!(
        state.settle_client_jumps(now, false),
        [
            ResolvedClientJump {
                origin: origin(),
                jump_id: "jump-1".into(),
                outcome: ClientJumpOutcome::Focused,
            },
            ResolvedClientJump {
                origin: from_build,
                jump_id: "jump-2".into(),
                outcome: ClientJumpOutcome::StaleBoot,
            },
        ]
    );
}

fn numbered(mut jump: EndpointClientJump, number: u64) -> EndpointClientJump {
    jump.jump_id = format!("jump-{number}");
    jump
}

fn focused(number: u64) -> Vec<ResolvedClientJump> {
    vec![ResolvedClientJump {
        origin: origin(),
        jump_id: format!("jump-{number}"),
        outcome: ClientJumpOutcome::Focused,
    }]
}

#[test]
fn a_newer_jump_replaces_the_pending_one_which_is_never_answered() {
    let (mut state, remote) = federated();
    let start = Instant::now();
    let first = state.receive_client_jump(
        origin(),
        jump(&remote.storage_key(), "build-boot", "pane_7"),
        start,
    );
    assert_activates(&first, &remote, "pane_7");
    assert!(state.settle_client_jumps(start, true).is_empty());

    // The daemon forwards another jump before the activation settles.
    let second = state.receive_client_jump(
        origin(),
        numbered(jump("local", "boot-1", "pane_2"), 2),
        start,
    );
    assert_activates(&second, &ClientEndpointId::Local, "pane_2");
    assert!(state.settle_client_jumps(start, true).is_empty());

    focus(&mut state, "pane_2");
    assert_eq!(state.settle_client_jumps(start, false), focused(2));
    // The replaced jump has no answer, not even a timeout: its daemon
    // answered its caller when it forwarded the second one.
    assert!(state
        .settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE * 2, false)
        .is_empty());
}

#[test]
fn a_jump_that_replaces_a_pending_one_navigates_even_to_the_pane_already_presented() {
    let mut state = single();
    let start = Instant::now();
    let first = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);
    let (_, first_request) = focus_request(&first, &ClientEndpointId::Local, "pane_2");

    // The second jump names the pane the client still presents. The first
    // jump's focus request is on its way, so the second sends one after it.
    let second = state.receive_client_jump(
        origin(),
        numbered(jump("local", "boot-1", "pane_1"), 2),
        start,
    );

    let (_, second_request) = focus_request(&second, &ClientEndpointId::Local, "pane_1");
    assert_ne!(first_request, second_request);
    // The pane shown in the turn that sent the request predates it.
    assert!(state.settle_client_jumps(start, false).is_empty());
    assert_eq!(state.settle_client_jumps(start, false), focused(2));
    // The first jump is never answered, whatever its navigation still does.
    focus(&mut state, "pane_2");
    focus(&mut state, "pane_1");
    assert!(state
        .settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE * 2, false)
        .is_empty());
}

#[test]
fn a_jump_to_the_pane_still_shown_navigates_in_place_of_the_replaced_jumps_activation() {
    let (mut state, remote) = federated();
    let start = Instant::now();
    let first = state.receive_client_jump(
        origin(),
        jump(&remote.storage_key(), "build-boot", "pane_7"),
        start,
    );
    assert_activates(&first, &remote, "pane_7");
    assert!(state.settle_client_jumps(start, true).is_empty());

    // Build's activation is in flight and Local is still shown. Answering
    // without navigating would let that activation complete, so the jump
    // selects Local as a click would, which the client loop puts in the
    // activation's place.
    let second = state.receive_client_jump(
        origin(),
        numbered(jump("local", "boot-1", "pane_1"), 2),
        start,
    );

    assert_activates(&second, &ClientEndpointId::Local, "pane_1");
    // The pane is presented, but the switch is not over yet.
    assert!(state.settle_client_jumps(start, true).is_empty());
    assert!(state.settle_client_jumps(start, true).is_empty());
    // Once it is, Local still shows the pane.
    assert_eq!(state.settle_client_jumps(start, false), focused(2));
    assert!(state
        .settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE * 2, false)
        .is_empty());
}

#[test]
fn a_jump_to_the_remote_pane_still_shown_navigates_after_the_replaced_jump() {
    let (mut state, remote) = federated();
    assert!(state.activate_endpoint_projection(&remote));
    let start = Instant::now();
    let first = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);
    assert_activates(&first, &ClientEndpointId::Local, "pane_2");
    assert!(state.settle_client_jumps(start, true).is_empty());

    // Build is still shown with `pane_1` focused. The jump navigates there
    // as a click on the agent would: a focus request to the endpoint shown.
    let second = state.receive_client_jump(
        origin(),
        numbered(jump(&remote.storage_key(), "build-boot", "pane_1"), 2),
        start,
    );
    focus_request(&second, &remote, "pane_1");

    // The switch to Local goes on, and the loop drops the request it cannot
    // send: the pane shown meanwhile is not the jump's doing.
    assert!(state.settle_client_jumps(start, true).is_empty());
    // The switch ends on Local, so the jump never sees its pane.
    assert!(state.activate_endpoint_projection(&ClientEndpointId::Local));
    assert!(state.settle_client_jumps(start, false).is_empty());
    assert_eq!(
        state.settle_client_jumps(start + NAVIGATION_SETTLE_DEADLINE, false),
        vec![ResolvedClientJump {
            origin: origin(),
            jump_id: "jump-2".into(),
            outcome: ClientJumpOutcome::Timeout,
        }]
    );
}

#[test]
fn no_jump_is_focused_while_a_switch_is_under_way_even_with_its_pane_shown() {
    let (mut state, remote) = federated();
    assert!(state.activate_endpoint_projection(&remote));
    let start = Instant::now();
    let key = remote.storage_key();
    let first = state.receive_client_jump(origin(), jump("local", "boot-1", "pane_2"), start);
    assert_activates(&first, &ClientEndpointId::Local, "pane_2");
    let second = state.receive_client_jump(
        origin(),
        numbered(jump(&key, "build-boot", "pane_1"), 2),
        start,
    );
    focus_request(&second, &remote, "pane_1");

    // Build shows the pane on every turn while Local's activation runs.
    for turn in 1..=3 {
        let now = start + Duration::from_millis(100 * turn);
        assert!(
            state.settle_client_jumps(now, true).is_empty(),
            "turn {turn}"
        );
    }
    // The activation gave way and Build is still presented: that is the
    // answer, on the first turn with no switch under way.
    assert_eq!(
        state.settle_client_jumps(start + Duration::from_secs(1), false),
        focused(2)
    );
}

#[test]
fn a_replacing_jump_on_the_presented_remote_is_a_focus_request_without_an_activation() {
    let (mut state, remote) = federated();
    assert!(state.activate_endpoint_projection(&remote));
    let start = Instant::now();
    let key = remote.storage_key();
    let first = state.receive_client_jump(origin(), jump(&key, "build-boot", "pane_7"), start);
    focus_request(&first, &remote, "pane_7");

    // Both jumps are requests to the endpoint shown, which applies them in
    // the order they were sent.
    let second = state.receive_client_jump(
        origin(),
        numbered(jump(&key, "build-boot", "pane_1"), 2),
        start,
    );

    focus_request(&second, &remote, "pane_1");
    assert!(state.settle_client_jumps(start, false).is_empty());
    assert_eq!(state.settle_client_jumps(start, false), focused(2));
}
