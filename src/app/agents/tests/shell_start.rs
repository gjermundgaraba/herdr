//! Telling a starting shell from a running command for `agent.start`.

#[test]
fn a_shell_leading_its_job_is_starting_and_anything_else_is_a_command() {
    let process = |pid, name: &str| crate::platform::ForegroundProcess {
        pid,
        name: name.into(),
        argv0: None,
        argv: None,
        cmdline: None,
    };
    let job = |process_group_id, processes| crate::platform::ForegroundJob {
        process_group_id,
        processes,
    };
    let starting = job(10, vec![process(10, "zsh"), process(11, "git")]);
    let exec_replaced = job(10, vec![process(10, "sleep")]);
    let command = job(20, vec![process(20, "vim")]);
    assert!(super::super::shell_is_starting(10, &starting));
    assert!(!super::super::shell_is_starting(10, &exec_replaced));
    assert!(!super::super::shell_is_starting(10, &command));
}
