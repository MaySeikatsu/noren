//! e2e: zellij operations — in-place switching (plugin pipe, native
//! fallback, error hint), kill-timeouts and readiness polling. zellij is a
//! stub script that logs its argv to $HOME/zellij-args.log.

mod common;
use common::*;

use std::fs;
use std::time::{Duration, Instant};

const INSIDE: &[(&str, &str)] = &[("ZELLIJ", "0"), ("ZELLIJ_SESSION_NAME", "cur")];

/// Logging stub: `body` is a `case "$1 $2"` arm list; unmatched calls exit 0.
fn stub(env: &Env, body: &str) {
    env.stub(
        "zellij",
        &format!(
            r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1 $2" in
{body}
esac
exit 0"#
        ),
    );
}

fn install_plugin(env: &Env) {
    let dir = env.home.join(".config/zellij/plugins");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("zellij-switch.wasm"), "").unwrap();
}

fn pipe_line(env: &Env, name: &str) -> String {
    format!(
        "pipe --plugin file:{}/.config/zellij/plugins/zellij-switch.wasm -- --session {name}",
        env.home.display()
    )
}

const TWO_LIVE: &str =
    r#"  "list-sessions "*) echo "cur [Created 1h ago]"; echo "foo [Created 2h ago]";;"#;

#[test]
fn switch_uses_plugin_pipe_when_present() {
    let env = Env::new();
    stub(&env, TWO_LIVE);
    install_plugin(&env);
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    assert_eq!(log.last().unwrap(), &pipe_line(&env, "foo"));
    assert!(!log.iter().any(|l| l.contains("switch-session")), "{log:?}");
}

#[test]
fn switch_falls_back_to_native_when_plugin_missing() {
    let env = Env::new();
    stub(&env, TWO_LIVE);
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    assert_eq!(log.last().unwrap(), "action switch-session foo");
    assert!(!log.iter().any(|l| l.starts_with("pipe")), "{log:?}");
}

#[test]
fn switch_falls_back_to_native_when_pipe_fails() {
    let env = Env::new();
    stub(&env, &format!("{TWO_LIVE}\n  \"pipe \"*) exit 1;;"));
    install_plugin(&env);
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    let pi = log
        .iter()
        .position(|l| l == &pipe_line(&env, "foo"))
        .unwrap();
    let ni = log
        .iter()
        .position(|l| l == "action switch-session foo")
        .unwrap();
    assert!(pi < ni, "{log:?}");
}

#[test]
fn switch_errors_with_hint_when_both_fail() {
    let env = Env::new();
    stub(
        &env,
        &format!("{TWO_LIVE}\n  \"pipe \"*) exit 1;;\n  \"action switch-session\") exit 1;;"),
    );
    install_plugin(&env);
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(!ok, "connect must fail loudly");
    assert!(err.contains("could not switch to session \"foo\""), "{err}");
    assert!(err.contains("zellij-switch.wasm"), "hint missing: {err}");
    // Never nests an attach from inside zellij.
    assert!(
        !env.zellij_log().iter().any(|l| l.starts_with("attach")),
        "{:?}",
        env.zellij_log()
    );
}

#[test]
fn native_switch_that_moves_no_client_is_an_error() {
    // zellij 0.45.1: `action switch-session` exits 0 from the CLI but moves
    // nobody. Our session keeps its one client → detected, reported.
    let env = Env::new();
    stub(
        &env,
        &format!(
            "{TWO_LIVE}\n  \"action list-clients\") echo \"CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\"; echo \"1 terminal_0 N/A\";;"
        ),
    );
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(!ok);
    assert!(err.contains("no client moved"), "{err}");
    assert!(err.contains("zellij-switch.wasm"), "{err}");
}

#[test]
fn native_switch_confirmed_by_client_leaving() {
    let env = Env::new();
    stub(
        &env,
        &format!(
            r#"{TWO_LIVE}
  "action switch-session") touch "$HOME/switched";;
  "action list-clients")
    echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
    [ -e "$HOME/switched" ] || echo "1 terminal_0 N/A";;"#
        ),
    );
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(ok, "stderr: {err}");
}

#[test]
fn hanging_pipe_times_out_and_falls_back() {
    let env = Env::new();
    // A pipe to a plugin that never answers (missing file / pending
    // permission prompt) blocks forever on real zellij.
    stub(&env, &format!("{TWO_LIVE}\n  \"pipe \"*) exec sleep 30;;"));
    install_plugin(&env);
    let start = Instant::now();
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    let took = start.elapsed();
    assert!(ok, "stderr: {err}");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    assert_eq!(
        env.zellij_log().last().unwrap(),
        "action switch-session foo"
    );
}

#[test]
fn native_fallback_resurrects_exited_target_first() {
    let env = Env::new();
    // foo is EXITED until `attach --create-background foo` resurrects it.
    stub(
        &env,
        r#"  "list-sessions "*)
    echo "cur [Created 1h ago]"
    if [ -e "$HOME/up" ]; then echo "foo [Created 0s ago]"
    else echo "foo [Created 2h ago] (EXITED - attach to resurrect)"; fi;;
  "attach --create-background") touch "$HOME/up";;"#,
    );
    let (_, err, ok) = env.run_env(&["connect", "foo"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    let ri = log
        .iter()
        .position(|l| l == "attach --create-background foo")
        .unwrap_or_else(|| panic!("no resurrect: {log:?}"));
    let si = log
        .iter()
        .position(|l| l == "action switch-session foo")
        .unwrap();
    assert!(ri < si, "{log:?}");
}

#[test]
fn connect_to_current_session_is_a_noop_switch() {
    let env = Env::new();
    stub(&env, TWO_LIVE);
    let (_, err, ok) = env.run_env(&["connect", "cur"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    assert!(
        !log.iter()
            .any(|l| l.starts_with("pipe") || l.contains("switch-session")),
        "{log:?}"
    );
}

#[test]
fn creation_polls_until_listed_then_switches() {
    let env = Env::new();
    stub(
        &env,
        r#"  "list-sessions "*)
    echo "cur [Created 1h ago]"
    [ -e "$HOME/created" ] && echo "fresh [Created 0s ago]";;
  "attach --create-background") touch "$HOME/created";;"#,
    );
    install_plugin(&env);
    let start = Instant::now();
    let (_, err, ok) = env.run_env(&["connect", "fresh"], INSIDE);
    assert!(ok, "stderr: {err}");
    // Listed on the first poll → no fixed sleep, no poll timeout.
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    let log = env.zellij_log();
    let ci = log
        .iter()
        .position(|l| l == "attach --create-background fresh")
        .unwrap();
    let si = log
        .iter()
        .position(|l| l == &pipe_line(&env, "fresh"))
        .unwrap();
    assert!(ci < si, "{log:?}");
}

#[test]
fn discard_waits_for_the_client_to_leave_before_killing() {
    let env = Env::new();
    // The client leaves `cur` only once the pipe ran; kill must come after
    // list-clients reported it gone.
    stub(
        &env,
        r#"  "list-sessions "*) echo "cur [Created 1h ago]"; echo "other [Created 2h ago]";;
  "pipe "*) touch "$HOME/switched";;
  "action list-clients")
    echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
    if [ -e "$HOME/switched" ]; then echo gone >> "$HOME/gone.log"
    else echo "1 terminal_0 N/A"; fi;;"#,
    );
    install_plugin(&env);
    let (out, err, ok) = env.run_env(&["discard"], INSIDE);
    assert!(ok, "stderr: {err}");
    assert_eq!(out, "discarded: cur\n");
    assert!(env.home.join("gone.log").exists(), "never polled clients");
    let log = env.zellij_log();
    let pi = log
        .iter()
        .position(|l| l == &pipe_line(&env, "other"))
        .unwrap();
    let ki = log.iter().position(|l| l == "kill-session cur").unwrap();
    assert!(pi < ki, "{log:?}");
}

#[test]
fn snapshot_dump_layout_times_out_and_falls_back_to_cache() {
    let env = Env::new();
    stub(
        &env,
        r#"  "list-sessions "*) echo "foo [Created 1h ago]";;
  "action dump-layout") exec sleep 30;;"#,
    );
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/foo");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("session-layout.kdl"), "layout { // cached\n}\n").unwrap();

    let start = Instant::now();
    let (out, err, ok) = env.run(&["snapshot", "foo"]);
    let took = start.elapsed();
    assert!(ok, "{out} {err}");
    assert!(took < Duration::from_secs(10), "took {took:?}");
    let dir = env.home.join(".local/state/noren/snapshots/foo");
    let snap = fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
    assert!(fs::read_to_string(snap).unwrap().contains("// cached"));
}

#[test]
fn window_refuses_sessions_that_are_not_running() {
    let env = Env::new();
    stub(
        &env,
        r#"  "list-sessions "*) echo "old [Created 1h ago] (EXITED - attach to resurrect)";;"#,
    );
    let (_, err, ok) = env.run(&["window", "--session", "old"]);
    assert!(!ok);
    assert!(err.contains("not running"), "{err}");
    // Never sends an action to an exited session (it would hang forever).
    assert!(
        !env.zellij_log().iter().any(|l| l.starts_with("action")),
        "{:?}",
        env.zellij_log()
    );
}

#[test]
fn window_action_on_hung_session_times_out() {
    let env = Env::new();
    stub(
        &env,
        r#"  "list-sessions "*) echo "foo [Created 1h ago]";;
  "action "*) exec sleep 30;;"#,
    );
    let start = Instant::now();
    let (_, _, ok) = env.run(&["window", "--session", "foo"]);
    let took = start.elapsed();
    assert!(ok);
    assert!(took < Duration::from_secs(10), "took {took:?}");
}

/// Stub where `attach --create-background <name> ...` makes <name> live.
const CREATES: &str = r#"  "list-sessions "*)
    echo "cur [Created 1h ago]"
    [ -e "$HOME/created" ] && cat "$HOME/created";;
  "attach --create-background") echo "$3 [Created 0s ago]" > "$HOME/created";;"#;

#[test]
fn connect_inside_with_layout_creates_in_background_with_default_layout() {
    let env = Env::new();
    stub(&env, CREATES);
    install_plugin(&env);
    fs::create_dir_all(env.home.join(".config/noren")).unwrap();
    fs::write(
        env.home.join(".config/noren/config.toml"),
        "[[session]]\nname = \"proj\"\nlayout = \"ide\"\n",
    )
    .unwrap();
    let layouts = env.home.join(".config/zellij/layouts");
    fs::create_dir_all(&layouts).unwrap();
    fs::write(layouts.join("ide.kdl"), "layout {\n}\n").unwrap();

    let (_, err, ok) = env.run_env(&["connect", "proj"], INSIDE);
    assert!(ok, "stderr: {err}");
    let log = env.zellij_log();
    // Not `--new-session-with-layout L -- true` (rejected by zellij 0.45).
    let create = format!(
        "attach --create-background proj options --default-layout {}",
        layouts.join("ide.kdl").display()
    );
    let ci = log
        .iter()
        .position(|l| l == &create)
        .unwrap_or_else(|| panic!("{log:?}"));
    let si = log
        .iter()
        .position(|l| l == &pipe_line(&env, "proj"))
        .unwrap();
    assert!(ci < si, "{log:?}");
}

#[test]
fn restore_inside_recreates_from_snapshot_then_switches() {
    let env = Env::new();
    stub(&env, CREATES);
    install_plugin(&env);
    let dir = env.home.join(".local/state/noren/snapshots/foo");
    fs::create_dir_all(&dir).unwrap();
    let snap = dir.join("20260713-090000.kdl");
    fs::write(&snap, "layout { }\n").unwrap();

    let (out, err, ok) = env.run_env(&["restore", "foo", "--force"], INSIDE);
    assert!(ok, "stderr: {err}");
    assert!(out.starts_with("restored: foo"), "{out}");
    let log = env.zellij_log();
    let create = format!(
        "attach --create-background foo options --default-layout {}",
        snap.display()
    );
    let ci = log
        .iter()
        .position(|l| l == &create)
        .unwrap_or_else(|| panic!("{log:?}"));
    let si = log
        .iter()
        .position(|l| l == &pipe_line(&env, "foo"))
        .unwrap();
    assert!(ci < si, "{log:?}");
}

#[test]
fn restore_refuses_the_session_it_runs_in() {
    let env = Env::new();
    env.stub_zellij_logging();
    let snaps = env.home.join(".local/state/noren/snapshots/cur");
    std::fs::create_dir_all(&snaps).unwrap();
    std::fs::write(snaps.join("20260101-000000.kdl"), "layout {}\n").unwrap();
    let (_, err, ok) = env.run_env(
        &["restore", "cur", "--force"],
        &[("ZELLIJ", "1"), ("ZELLIJ_SESSION_NAME", "cur")],
    );
    assert!(!ok);
    assert!(err.contains("from inside itself"), "{err}");
    assert!(
        !env.zellij_log()
            .iter()
            .any(|l| l.contains("kill-session") || l.contains("delete-session")),
        "nothing may be killed: {:?}",
        env.zellij_log()
    );
}
