//! The fork's OSC 7501 in-band state reports (`state/in_band.rs`).

use super::*;

#[test]
fn pi_in_band_state_follows_the_session_the_integration_reported() {
    let mut terminal = test_terminal();
    let session = test_session_path("pi-in-band.jsonl");
    // Pi reports its session before the process probe recognizes it.
    terminal.set_agent_session_ref_for_session_start(
        "herdr:pi".into(),
        "pi".into(),
        crate::agent_resume::AgentSessionRef::path(session.clone()),
        Some(10),
        Some("startup".into()),
    );
    terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);

    let working = terminal.set_in_band_hook_state_at("pi", AgentState::Working, Instant::now());

    assert!(working.is_some());
    assert_eq!(terminal.state, AgentState::Working);
    assert_eq!(
        terminal.current_session_identity_for_persistence(),
        Some((
            "herdr:pi".into(),
            "pi".into(),
            crate::agent_resume::AgentSessionRefKind::Path,
            session,
        ))
    );
    // In-band reports leave the socket's sequence to the integration.
    assert_eq!(terminal.hook_report_sequences.get("herdr:pi"), Some(&10));

    terminal.set_in_band_hook_state_at("pi", AgentState::Blocked, Instant::now());
    assert_eq!(terminal.state, AgentState::Blocked);

    let replacement = test_session_path("pi-in-band-new.jsonl");
    let new_session = terminal.set_agent_session_ref_for_session_start(
        "herdr:pi".into(),
        "pi".into(),
        crate::agent_resume::AgentSessionRef::path(replacement.clone()),
        Some(11),
        Some("new".into()),
    );
    assert!(new_session.is_some());
    terminal.set_in_band_hook_state_at("pi", AgentState::Idle, Instant::now());
    assert_eq!(terminal.state, AgentState::Idle);
    assert_eq!(
        terminal.hook_authority.as_ref().unwrap().session_ref,
        crate::agent_resume::AgentSessionRef::path(replacement)
    );
    // Moving to the replacement session must not reset the integration's sequence.
    assert_eq!(terminal.hook_report_sequences.get("herdr:pi"), Some(&11));
}

#[test]
fn in_band_state_applies_only_to_the_reported_agent() {
    let mut terminal = test_terminal();
    anchor_full_lifecycle_session(
        &mut terminal,
        Agent::Pi,
        "herdr:pi",
        "pi",
        crate::agent_resume::AgentSessionRef::path(test_session_path("pi-in-band-app.jsonl"))
            .unwrap(),
    );

    let other = terminal.set_in_band_hook_state_at("omp", AgentState::Working, Instant::now());

    assert!(other.is_none());
    assert!(terminal.hook_authority.is_none());
}

#[test]
fn in_band_state_applies_to_any_state_reporting_integration_by_agent_name() {
    let mut terminal = test_terminal();
    anchor_full_lifecycle_session(
        &mut terminal,
        Agent::Omp,
        "herdr:omp",
        "omp",
        crate::agent_resume::AgentSessionRef::path(test_session_path("omp-in-band.jsonl")).unwrap(),
    );

    let working = terminal.set_in_band_hook_state_at("omp", AgentState::Working, Instant::now());

    assert!(working.is_some());
    assert_eq!(terminal.state, AgentState::Working);
    assert_eq!(
        terminal.hook_authority.as_ref().unwrap().source,
        "herdr:omp"
    );
}

#[test]
fn pi_in_band_state_resumes_after_pi_restarts_in_the_pane() {
    let mut terminal = test_terminal();
    let first = test_session_path("pi-in-band-first.jsonl");
    let second = test_session_path("pi-in-band-second.jsonl");
    anchor_full_lifecycle_session(
        &mut terminal,
        Agent::Pi,
        "herdr:pi",
        "pi",
        crate::agent_resume::AgentSessionRef::path(first).unwrap(),
    );
    let start = Instant::now();
    terminal.set_in_band_hook_state_at("pi", AgentState::Working, start);
    let exited = start + Duration::from_secs(1);
    terminal.set_detected_state_with_screen_signals_at(
        Some(Agent::Pi),
        AgentState::Idle,
        false,
        true,
        false,
        true,
        exited,
    );

    terminal.set_agent_session_ref_for_session_start(
        "herdr:pi".into(),
        "pi".into(),
        crate::agent_resume::AgentSessionRef::path(second.clone()),
        Some(20),
        Some("startup".into()),
    );
    let restarted = exited + Duration::from_secs(1);
    terminal.set_detected_state_with_screen_signals_at(
        Some(Agent::Pi),
        AgentState::Idle,
        false,
        false,
        false,
        false,
        restarted,
    );
    let working = terminal.set_in_band_hook_state_at(
        "pi",
        AgentState::Working,
        restarted + Duration::from_millis(100),
    );

    assert!(working.is_some());
    assert_eq!(terminal.state, AgentState::Working);
    assert_eq!(
        terminal.hook_authority.as_ref().unwrap().session_ref,
        crate::agent_resume::AgentSessionRef::path(second)
    );
}

#[test]
fn pi_in_band_state_waits_for_a_reported_session() {
    let mut terminal = test_terminal();
    terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);

    let working = terminal.set_in_band_hook_state_at("pi", AgentState::Working, Instant::now());

    assert!(working.is_none());
    assert!(terminal.hook_authority.is_none());
}
