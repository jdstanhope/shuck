use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn huck_binary() -> String {
    env!("CARGO_BIN_EXE_huck").to_string()
}

/// Runs huck with `script` on stdin, captures stdout/stderr, returns
/// (stdout, stderr, exit_status).
fn run(script: &str) -> (String, String, std::process::ExitStatus) {
    let mut child = Command::new(huck_binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn huck");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status,
    )
}

/// Spawns huck with `script`, returns the child handle (still running)
/// + the pid. Caller is responsible for finishing the process.
fn spawn(script: &str) -> (std::process::Child, i32) {
    let mut child = Command::new(huck_binary())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn huck");
    let pid = child.id() as i32;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    // No — we want huck to KEEP running until we send the signal.
    // Don't drop; the caller will manage.
    (child, pid)
}

/// Sends `signum` to `pid` via libc::kill.
fn send_signal(pid: i32, signum: i32) {
    unsafe {
        libc::kill(pid, signum);
    }
}

#[test]
fn exit_trap_fires_on_normal_exit() {
    let (out, _err, status) = run("trap 'echo bye' EXIT\nexit 0\n");
    assert!(out.lines().any(|l| l == "bye"), "stdout: {out}");
    assert_eq!(status.code(), Some(0));
}

#[test]
fn exit_trap_sees_last_status() {
    let (out, _err, _) = run("trap 'echo dollar-q=$?' EXIT\nfalse\nexit\n");
    assert!(out.lines().any(|l| l == "dollar-q=1"), "stdout: {out}");
}

#[test]
fn exit_trap_fires_on_eof() {
    // Script ends without explicit `exit`. EOF should still fire EXIT.
    let (out, _err, _) = run("trap 'echo bye' EXIT\n");
    assert!(out.lines().any(|l| l == "bye"), "stdout: {out}");
}

#[test]
fn exit_trap_fires_only_once() {
    // Recursive exit from within the action should NOT re-fire.
    let (out, _err, _) = run("trap 'echo bye; exit 0' EXIT\nexit 1\n");
    let bye_count = out.lines().filter(|l| **l == *"bye").count();
    assert_eq!(bye_count, 1, "stdout: {out}");
}

#[test]
fn exit_trap_cleared_in_subshell() {
    // Parent's EXIT fires only once when the parent exits. Subshell
    // does NOT fire it again.
    let (out, _err, _) = run("trap 'echo parent-bye' EXIT\n(echo child)\nexit\n");
    let bye_count = out.lines().filter(|l| **l == *"parent-bye").count();
    assert_eq!(bye_count, 1, "stdout: {out}");
    assert!(out.lines().any(|l| l == "child"), "stdout: {out}");
}

#[test]
fn trap_dash_resets_exit() {
    // Set, then reset. EXIT trap should NOT fire.
    let (out, _err, _) = run("trap 'echo bye' EXIT\ntrap - EXIT\nexit 0\n");
    assert!(!out.lines().any(|l| l == "bye"), "stdout: {out}");
}

#[test]
fn trap_empty_action_ignores_exit() {
    // Empty action = ignore. EXIT does not run anything.
    let (out, _err, _) = run("trap '' EXIT\nexit 0\n");
    // No specific output to assert non-presence of — but exit must succeed.
    assert!(!out.contains("bye"));
}

#[test]
fn trap_p_output_format() {
    let (out, _err, _) = run("trap 'echo bye' EXIT\ntrap -p\nexit\n");
    // The `trap -p` output should appear before the EXIT action runs.
    assert!(
        out.lines().any(|l| l == "trap -- 'echo bye' EXIT"),
        "stdout: {out}"
    );
}

#[test]
fn trap_l_lists_signals() {
    let (out, _err, _) = run("trap -l\nexit\n");
    assert!(out.contains("2) SIGINT"), "stdout: {out}");
    assert!(out.contains("15) SIGTERM"), "stdout: {out}");
}

#[test]
fn trap_kill_accepted_silently_and_listed() {
    // bash does NOT reject `trap … KILL`: it silently accepts (no error) and
    // stores the disposition (visible via `trap -p`), though it never fires.
    let (_out, err, _) = run("trap 'echo nope' KILL\nexit 0\n");
    assert!(
        !err.contains("cannot trap"),
        "should not error; stderr: {err}"
    );
    assert!(err.is_empty(), "no stderr expected; got: {err}");
    let (out, _err, _) = run("trap 'echo nope' KILL\ntrap -p KILL\n");
    assert!(
        out.contains("echo nope") && out.contains("KILL"),
        "trap -p should list the KILL disposition; stdout: {out}"
    );
}

#[test]
fn trap_unknown_signal_errors_exit_1() {
    let (_out, err, _) = run("trap 'echo nope' NOPE\nexit 0\n");
    assert!(
        err.contains("invalid signal specification"),
        "stderr: {err}"
    );
}

#[test]
fn sigint_trap_fires_action() {
    // The script ANNOUNCES readiness and the test waits for it, rather than
    // guessing with a fixed sleep (#749).
    //
    // ⚠️ The window this closes is real: everything between spawn and the
    // signal — huck starting, reading the script, installing the trap, and
    // entering `sleep` — has to have happened, or SIGINT lands on the DEFAULT
    // disposition and kills huck outright, with `caught` never printed and an
    // empty stdout. The old 200 ms guess held in the full workspace sweep but
    // failed 4/4 on macOS when this binary was run on its own, which is the
    // normal thing to do while working on traps.
    //
    // `echo READY` runs after the `trap` builtin and before `sleep`, so seeing
    // it on stdout proves the trap is installed. Reading to EOF without it
    // means huck died early — reported as such rather than as a missing
    // `caught`, which is what made the old failure mode opaque.
    let (mut child, pid) = spawn("trap 'echo caught' INT\necho READY\nsleep 2\nexit\n");
    // Drop stdin so huck reads to EOF and runs the script body.
    drop(child.stdin.take());

    let mut out = child.stdout.take().expect("stdout is piped");
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !String::from_utf8_lossy(&seen).contains("READY") {
        match out.read(&mut byte) {
            // EOF before READY: huck exited instead of reaching `sleep`.
            Ok(0) => panic!(
                "huck exited before printing READY; stdout so far: {:?}",
                String::from_utf8_lossy(&seen)
            ),
            Ok(_) => seen.extend_from_slice(&byte),
            Err(e) => panic!("reading huck's stdout failed: {e}"),
        }
    }

    send_signal(pid, libc::SIGINT);

    // Bounded: the trap action runs and huck reaches `exit`, closing stdout.
    let mut rest = String::new();
    out.read_to_string(&mut rest)
        .expect("read remaining stdout");
    let _ = child.wait();
    assert!(
        rest.contains("caught"),
        "the INT trap action did not run; stdout after READY: {rest:?}"
    );
}

#[test]
fn trap_in_function_persists_after_return() {
    // trap is shell-global, not function-local.
    let script = "f() { trap 'echo bye' EXIT; }\nf\nexit 0\n";
    let (out, _err, _) = run(script);
    assert!(out.lines().any(|l| l == "bye"), "stdout: {out}");
}

#[test]
fn a_forking_err_trap_action_does_not_disturb_the_surrounding_status() {
    // The ERR action's own `$?` (7 here) must not leak into the status the
    // script observes after the failing command — that stays 42.
    //
    // ⚠️ This lives here, not beside `run_trap_action`'s unit tests, because the
    // action FORKS: `(exit 7)` is a subshell, and so is the `(exit 42)` that
    // triggers it. huck runs subshells by forking without exec, which is only
    // safe in a single-threaded process, so `assert_single_threaded_fork()`
    // panics if any other thread is executing shell code. In the shared lib test
    // binary ~2000 tests run concurrently and that guard fires (macOS loses the
    // race every time; Linux usually wins it) — see #747. Each test here spawns
    // its OWN huck process, which is single-threaded, so forking is safe.
    //
    // The unit test keeps the same two assertions with a non-forking action;
    // this row is what preserves coverage of a SUBSHELL action end to end.
    // Verified byte-identical against bash 5.2.21.
    let script = "trap \"(exit 7)\" ERR\n(exit 42)\necho \"after=$?\"\n";
    let (out, _err, _) = run(script);
    assert!(
        out.lines().any(|l| l == "after=42"),
        "the trap action's status leaked into the surrounding $?; stdout: {out}"
    );
}
