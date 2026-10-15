//! Picker binds (`noren pick <event>` transform handlers), kill/delete
//! confirmation policy, and config diagnostics.

use std::fs;

mod common;
use common::*;

/// Sessions: `att` (live, a terminal attached), `bg` (live, background),
/// `dead` (exited). Every call is logged to $HOME/zellij-args.log.
const ZELLIJ: &str = r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1" in
  list-sessions) printf 'att [Created 1h ago]\nbg [Created 1h ago]\ndead [Created 1h ago] (EXITED - attach to resurrect)\n';;
  action)
    if [ "$2" = list-clients ]; then
      echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
      [ "$ZELLIJ_SESSION_NAME" = att ] && echo "1 terminal_0 hx"
    fi;;
esac
exit 0"#;

fn setup() -> Env {
    let env = Env::new();
    env.stub("zellij", ZELLIJ);
    env.stub("zoxide", "echo /tmp");
    env
}

/// zellij calls that change something (no list/query noise).
fn mutations(env: &Env) -> Vec<String> {
    env.zellij_log()
        .into_iter()
        .filter(|l| l.starts_with("kill-session") || l.starts_with("delete-session"))
        .collect()
}

fn row(source: &str, name: &str, state: &str) -> String {
    format!("{source}\t{name}\t\t{state}\t{name}")
}

/// Run `noren pick` as fzf would: picker id, runtime dir, prompt, query.
fn pick(env: &Env, args: &[&str], prompt: &str, query: &str) -> String {
    let rt = env.home.join("run");
    fs::create_dir_all(&rt).unwrap();
    let mut full = vec!["pick"];
    full.extend_from_slice(args);
    let (out, err, ok) = env.run_env(
        &full,
        &[
            ("NOREN_PICKER_ID", "42"),
            ("XDG_RUNTIME_DIR", rt.to_str().unwrap()),
            ("FZF_PROMPT", prompt),
            ("FZF_QUERY", query),
        ],
    );
    assert!(ok, "pick {args:?} failed: {err}");
    out
}

// ---- CLI kill / delete -----------------------------------------------------

#[test]
fn kill_background_session_without_asking() {
    let env = setup();
    let (out, _, ok) = env.run(&["kill", "bg"]);
    assert!(ok);
    assert_eq!(out.trim(), "killed bg");
    assert_eq!(mutations(&env), ["kill-session bg"]);
}

#[test]
fn kill_exited_session_is_a_friendly_noop() {
    let env = setup();
    let (out, _, ok) = env.run(&["kill", "dead"]);
    assert!(ok);
    assert!(out.contains("not running"), "{out}");
    assert!(mutations(&env).is_empty());
}

#[test]
fn kill_unknown_session_fails() {
    let env = setup();
    let (_, err, ok) = env.run(&["kill", "nope"]);
    assert!(!ok);
    assert!(err.contains("no session named \"nope\""), "{err}");
}

#[test]
fn delete_uses_force_only_for_running_sessions() {
    let env = setup();
    assert!(env.run(&["delete", "dead"]).2);
    assert!(env.run(&["delete", "bg", "--yes"]).2);
    assert_eq!(
        mutations(&env),
        ["delete-session dead", "delete-session --force bg"]
    );
}

#[test]
fn killing_the_current_session_switches_away_first() {
    let env = setup();
    let (_, _, ok) = env.run_env(
        &["kill", "att", "--yes"],
        &[("ZELLIJ", "1"), ("ZELLIJ_SESSION_NAME", "att")],
    );
    assert!(ok);
    let log = env.zellij_log();
    let switch = log.iter().position(|l| l.contains("--session bg"));
    let kill = log.iter().position(|l| l == "kill-session att");
    assert!(switch.is_some() && kill.is_some(), "log: {log:?}");
    assert!(switch < kill, "must leave before killing: {log:?}");
}

// ---- picker binds ----------------------------------------------------------

#[test]
fn pick_kill_background_runs_immediately_and_keeps_the_view() {
    let env = setup();
    let out = pick(&env, &["kill", &row("zellij", "bg", "live")], "▢  ", "");
    assert_eq!(mutations(&env), ["kill-session bg"]);
    // Reloads the sessions view (prompt ▢), not `list all`.
    assert!(out.contains(" list zellij)"), "{out}");
    assert!(out.contains("change-ghost(✓ killed bg)"), "{out}");
}

#[test]
fn pick_kill_attached_asks_then_y_confirms() {
    let env = setup();
    let out = pick(&env, &["kill", &row("zellij", "att", "live")], "⚡  ", "");
    assert!(mutations(&env).is_empty(), "must not kill before the answer");
    assert!(out.contains("rebind(change)"), "{out}");
    let prompt = "⚠ kill attached session att? y/N ";
    assert!(out.contains(&format!("change-prompt({prompt})")), "{out}");

    // Typing `y` answers through the change handler.
    let out = pick(&env, &["answer"], prompt, "y");
    assert_eq!(mutations(&env), ["kill-session att"]);
    assert!(out.contains("unbind(change)"), "{out}");
    assert!(out.contains("change-prompt(⚡  )"), "{out}");
    assert!(out.contains(" list all)"), "{out}");
}

#[test]
fn pick_confirmation_cancels_on_other_keys_enter_and_esc() {
    let env = setup();
    let prompt = "⚠ delete running session bg? y/N ";
    for (event, query) in [("answer", "n"), ("enter", ""), ("esc", "")] {
        let out = pick(&env, &["delete", &row("zellij", "bg", "live")], "▢  ", "");
        assert!(out.contains(prompt), "{out}");
        let out = pick(&env, &[event], prompt, query);
        assert!(out.contains("delete cancelled"), "{event}: {out}");
        assert!(out.contains("change-prompt(▢  )"), "{event}: {out}");
        assert!(!out.contains("abort") && !out.contains("accept"), "{out}");
    }
    assert!(mutations(&env).is_empty());
}

#[test]
fn pick_answer_ignores_empty_query_and_stale_state() {
    let env = setup();
    pick(&env, &["delete", &row("zellij", "bg", "live")], "▢  ", "");
    let prompt = "⚠ delete running session bg? y/N ";
    assert_eq!(pick(&env, &["answer"], prompt, ""), "");
    // The user switched views meanwhile: the confirmation is gone.
    assert_eq!(pick(&env, &["answer"], "📁  ", "y"), "unbind(change)");
    assert!(mutations(&env).is_empty());
}

#[test]
fn pick_delete_exited_session_needs_no_confirmation() {
    let env = setup();
    let out = pick(&env, &["delete", &row("zellij", "dead", "exited")], "⚡  ", "");
    assert_eq!(mutations(&env), ["delete-session dead"]);
    assert!(out.contains("✓ deleted dead"), "{out}");
}

#[test]
fn pick_kill_on_a_folder_explains_instead_of_acting() {
    let env = setup();
    let out = pick(&env, &["kill", &row("zoxide", "tmp", "dir")], "⚡  ", "");
    assert!(out.starts_with("change-ghost(✗"), "{out}");
    assert!(mutations(&env).is_empty());
}

#[test]
fn pick_current_session_kill_hands_off_to_cli() {
    let env = setup();
    let rt = env.home.join("run");
    fs::create_dir_all(&rt).unwrap();
    let base = [
        ("NOREN_PICKER_ID", "7"),
        ("XDG_RUNTIME_DIR", rt.to_str().unwrap()),
        ("ZELLIJ", "1"),
        ("ZELLIJ_SESSION_NAME", "att"),
    ];
    let line = row("zellij", "att", "live");
    let mut e = base.to_vec();
    e.push(("FZF_PROMPT", "⚡  "));
    let (out, _, _) = env.run_env(&["pick", "kill", &line], &e);
    let prompt = "⚠ kill the session you are in att? y/N ";
    assert!(out.contains(prompt), "{out}");
    let mut e = base.to_vec();
    e.push(("FZF_PROMPT", prompt));
    e.push(("FZF_QUERY", "y"));
    let (out, _, _) = env.run_env(&["pick", "answer"], &e);
    assert!(out.contains("execute-silent("), "{out}");
    assert!(out.contains(" kill --yes 'att'"), "{out}");
    assert!(mutations(&env).is_empty(), "the CLI does the work");
}

#[test]
fn pick_enter_and_esc_without_confirmation() {
    let env = setup();
    let sep = "sep\t\t\tsep\t╌╌";
    assert_eq!(pick(&env, &["enter", sep], "⚡  ", ""), "ignore");
    assert_eq!(
        pick(&env, &["enter", &row("zellij", "bg", "live")], "⚡  ", ""),
        "accept-or-print-query"
    );
    assert_eq!(pick(&env, &["esc"], "⚡  ", ""), "abort");
}

#[test]
fn pick_pin_dir_reloads_current_view() {
    let env = setup();
    let line = "zoxide\ttmp\t/tmp\tdir\t📁 tmp";
    let out = pick(&env, &["pin", line], "📁  ", "");
    assert!(out.contains(" list zoxide)"), "{out}");
    assert!(out.contains("📌 pinned /tmp"), "{out}");
    let pins = fs::read_to_string(env.home.join(".local/state/noren/pinned-dirs")).unwrap();
    assert_eq!(pins, "/tmp\n");
}

#[test]
fn pick_reload_keeps_the_view() {
    let env = setup();
    let out = pick(&env, &["reload"], "⚙  ", "");
    assert!(out.starts_with("reload("), "{out}");
    assert!(out.contains(" list config)"), "{out}");
}

// ---- config diagnostics ----------------------------------------------------

fn write_config(env: &Env, body: &str) {
    fs::create_dir_all(env.home.join(".config/noren")).unwrap();
    fs::write(env.home.join(".config/noren/config.toml"), body).unwrap();
}

#[test]
fn unknown_keys_and_bad_values_are_reported() {
    let env = setup();
    write_config(
        &env,
        "sort_oder = []\npreview_mode = \"fancy\"\n[keys]\nkil = \"ctrl-k\"\n\
         [[session]]\nname = \"x\"\nlayuot = \"ide\"\n",
    );
    let (_, err, ok) = env.run(&["list", "config"]);
    assert!(ok);
    for needle in [
        "unknown config key `sort_oder`",
        "unknown config key `keys.kil`",
        "unknown config key `session[0].layuot`",
        "preview_mode = \"fancy\"",
    ] {
        assert!(err.contains(needle), "missing {needle:?} in: {err}");
    }
    // The picker's re-execs stay quiet (it shows these in its header).
    let (_, err, _) = env.run_env(&["list", "config"], &[("NOREN_QUIET", "1")]);
    assert!(err.is_empty(), "{err}");
}

#[test]
fn invalid_confirm_policy_is_a_parse_error_with_position() {
    let env = setup();
    write_config(&env, "[confirm]\nkill = \"sometimes\"\n");
    let (_, err, _) = env.run(&["list", "config"]);
    assert!(err.contains("config error in"), "{err}");
    assert!(err.contains("line 2"), "{err}");
}

#[test]
fn confirm_policy_is_configurable() {
    let env = setup();
    write_config(&env, "[confirm]\nkill = \"running\"\n");
    let out = pick(&env, &["kill", &row("zellij", "bg", "live")], "⚡  ", "");
    assert!(out.contains("⚠ kill running session bg? y/N "), "{out}");
    write_config(&env, "[confirm]\ndelete = \"never\"\n");
    let out = pick(&env, &["delete", &row("zellij", "att", "live")], "⚡  ", "");
    assert!(out.contains("✓ deleted att"), "{out}");
}
