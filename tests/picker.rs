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
    let plugins = env.home.join(".config/zellij/plugins");
    fs::create_dir_all(&plugins).unwrap();
    fs::write(plugins.join("zellij-switch.wasm"), "").unwrap();
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
    assert!(
        mutations(&env).is_empty(),
        "must not kill before the answer"
    );
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
    let out = pick(
        &env,
        &["delete", &row("zellij", "dead", "exited")],
        "⚡  ",
        "",
    );
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
    // Not -silent: a failed switch-away must stay readable.
    assert!(
        out.contains("execute(") && !out.contains("execute-silent"),
        "{out}"
    );
    assert!(out.contains(" kill --yes 'att' || "), "{out}");
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

#[test]
fn shipped_example_config_is_warning_free() {
    let env = setup();
    let example =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml.example")).unwrap();
    write_config(&env, &example);
    let (_, err, ok) = env.run(&["list", "config"]);
    assert!(ok);
    assert!(err.is_empty(), "example config warns: {err}");
}

#[test]
fn deleting_the_previous_session_forgets_it_for_last() {
    let env = setup();
    let state = env.home.join(".local/state/noren");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("previous"), "dead").unwrap();
    fs::write(state.join("last"), "bg").unwrap();
    assert!(env.run(&["delete", "dead"]).2);
    assert!(!state.join("previous").exists());
    // `last` now says so instead of recreating an empty "dead".
    let (_, err, ok) = env.run(&["last"]);
    assert!(!ok);
    assert!(err.contains("no previous session"), "{err}");
}

// ---- snapshot menu ---------------------------------------------------------

fn snap_dir(env: &Env, name: &str) -> std::path::PathBuf {
    let d = env.home.join(".local/state/noren/snapshots").join(name);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn ctrl_s_on_a_session_opens_its_snapshot_menu_and_esc_returns() {
    let env = setup();
    let out = pick(&env, &["snapshot", &row("zellij", "bg", "live")], "▢  ", "");
    assert!(out.contains("change-prompt(📸 bg  )"), "{out}");
    assert!(out.contains(" snapshot-rows 'bg')"), "{out}");
    assert!(out.contains("⏎ restore"), "{out}");
    // esc goes back to the view the menu was opened from.
    let out = pick(&env, &["esc"], "📸 bg  ", "");
    assert!(out.contains("change-prompt(▢  )"), "{out}");
    assert!(out.contains(" list zellij)"), "{out}");
    assert!(!out.contains("abort"), "{out}");
}

#[test]
fn ctrl_s_on_a_folder_explains() {
    let env = setup();
    let out = pick(
        &env,
        &["snapshot", "zoxide\ttmp\t/tmp\tdir\ttmp"],
        "⚡  ",
        "",
    );
    assert!(out.starts_with("change-ghost(✗"), "{out}");
}

#[test]
fn snapshot_rows_list_take_now_first_then_newest() {
    let env = setup();
    let d = snap_dir(&env, "bg");
    fs::write(d.join("20260101-000000.kdl"), "layout {\n tab {\n }\n}\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(d.join("20260102-000000.kdl"), "layout {\n}\n").unwrap();
    let (out, _, ok) = env.run(&["snapshot-rows", "bg"]);
    assert!(ok);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "snapnew\tbg\t\tsnapnew\t＋ take a snapshot of bg now"
    );
    assert!(
        lines[1].contains("20260102-000000.kdl\tsnapshot\t📸 #1  2026-01-02"),
        "{out}"
    );
    assert!(lines[2].contains("📸 #2  2026-01-01 00:00:00 UTC"), "{out}");
    assert!(lines[2].ends_with("· 1 tab"), "{out}");
}

#[test]
fn enter_on_take_now_saves_a_snapshot() {
    let env = Env::new();
    env.stub(
        "zellij",
        r#"case "$1" in
  list-sessions) echo "bg [Created 1h ago]";;
  action) [ "$2" = dump-layout ] && echo 'layout { tab { } }';;
esac"#,
    );
    let out = pick(
        &env,
        &["enter", "snapnew\tbg\t\tsnapnew\t＋"],
        "📸 bg  ",
        "",
    );
    assert!(out.contains("📸 saved"), "{out}");
    assert!(out.contains(" snapshot-rows 'bg')"), "{out}");
    assert_eq!(fs::read_dir(snap_dir(&env, "bg")).unwrap().count(), 1);
}

#[test]
fn restore_from_menu_confirms_then_hands_off_to_the_picker() {
    let env = setup();
    let f = snap_dir(&env, "bg").join("20260101-000000.kdl");
    fs::write(&f, "layout {\n}\n").unwrap();
    let line = format!("snapshot\tbg\t{}\tsnapshot\t📸 #1", f.display());
    let out = pick(&env, &["enter", &line], "📸 bg  ", "");
    let prompt = "⚠ restore bg to 20260101-000000? y/N ";
    assert!(out.contains(prompt), "{out}");
    let out = pick(&env, &["answer"], prompt, "y");
    assert!(
        out.contains(&format!("print(__noren_restore__\tbg\t{})", f.display())),
        "{out}"
    );
    assert!(out.ends_with("+accept"), "{out}");
    assert!(
        mutations(&env).is_empty(),
        "the picker restores after fzf exits"
    );
}

#[test]
fn restore_of_the_current_session_is_refused_in_the_menu() {
    let env = setup();
    let rt = env.home.join("run");
    fs::create_dir_all(&rt).unwrap();
    let line = "snapshot\tatt\t/x.kdl\tsnapshot\t📸 #1";
    let (out, _, _) = env.run_env(
        &["pick", "enter", line],
        &[
            ("NOREN_PICKER_ID", "3"),
            ("XDG_RUNTIME_DIR", rt.to_str().unwrap()),
            ("FZF_PROMPT", "📸 att  "),
            ("ZELLIJ", "1"),
            ("ZELLIJ_SESSION_NAME", "att"),
        ],
    );
    assert!(out.contains("✗ can't restore att from inside it"), "{out}");
}

#[test]
fn ctrl_d_on_a_snapshot_deletes_it_after_y() {
    let env = setup();
    let f = snap_dir(&env, "bg").join("20260101-000000.kdl");
    fs::write(&f, "layout {\n}\n").unwrap();
    let line = format!("snapshot\tbg\t{}\tsnapshot\t📸 #1", f.display());
    let out = pick(&env, &["delete", &line], "📸 bg  ", "");
    let prompt = "⚠ delete snapshot 20260101-000000? y/N ";
    assert!(out.contains(prompt), "{out}");
    let out = pick(&env, &["answer"], prompt, "y");
    assert!(!f.exists());
    assert!(out.contains("🗑 deleted snapshot 20260101-000000"), "{out}");
    assert!(
        out.contains("change-prompt(📸 bg  )"),
        "back in the menu: {out}"
    );
    assert!(mutations(&env).is_empty(), "no session was touched");
}

#[test]
fn no_match_in_the_snapshot_menu_does_not_create_a_session() {
    let env = setup();
    assert_eq!(pick(&env, &["enter", ""], "📸 bg  ", "zzz"), "ignore");
}

// ---- hidden entries --------------------------------------------------------

fn names_listed(env: &Env, extra: &[(&str, &str)]) -> Vec<String> {
    let (out, _, _) = env.run_env(&["list", "all"], extra);
    out.lines()
        .map(|l| l.split('\t').nth(1).unwrap_or("").to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

#[test]
fn ctrl_h_hides_and_alt_h_shows_hidden_entries_marked() {
    let env = setup();
    let out = pick(&env, &["hide", &row("zellij", "bg", "live")], "⚡  ", "");
    assert!(
        out.contains("🙈 hid bg · M-h shows hidden entries"),
        "{out}"
    );
    assert!(out.contains(" list all)"), "{out}");
    let out = pick(&env, &["hide", "zoxide\ttmp\t/tmp\tdir\ttmp"], "⚡  ", "");
    assert!(out.contains("🙈 hid /tmp"), "{out}");
    let hidden = fs::read_to_string(env.home.join(".local/state/noren/hidden")).unwrap();
    assert_eq!(hidden, "session\tbg\ndir\t/tmp\n");

    // Plain list (CLI, or the picker by default): gone.
    let names = names_listed(&env, &[]);
    assert!(!names.contains(&"bg".to_string()) && !names.contains(&"tmp".to_string()));
    // `noren list hidden` shows only them.
    let (out, _, _) = env.run(&["list", "hidden"]);
    let rows: Vec<&str> = out.lines().filter(|l| !l.starts_with("sep\t")).collect();
    assert_eq!(rows.len(), 2, "{out}");

    // Alt h inside a picker: listed again, marked ⊘ and dim italic.
    let rt = env.home.join("run");
    let out = pick(&env, &["show-hidden"], "⚡  ", "");
    assert!(out.contains("showing hidden entries"), "{out}");
    assert!(rt.join("noren-pick-42.hidden").exists());
    let (out, _, _) = env.run_env(
        &["list", "all"],
        &[
            ("NOREN_PICKER_ID", "42"),
            ("XDG_RUNTIME_DIR", rt.to_str().unwrap()),
        ],
    );
    let bg = out
        .lines()
        .find(|l| l.starts_with("zellij\tbg\t"))
        .expect("bg listed");
    assert!(bg.contains("⊘ \x1b[2;3mbg\x1b[0m"), "{bg:?}");
    let att = out
        .lines()
        .find(|l| l.starts_with("zellij\tatt\t"))
        .unwrap();
    assert!(!att.contains('⊘'), "{att:?}");

    // ^h on a hidden row unhides it; Alt h again stops showing hidden ones.
    let out = pick(&env, &["hide", &row("zellij", "bg", "live")], "⚡  ", "");
    assert!(out.contains("👁 bg is visible again"), "{out}");
    pick(&env, &["show-hidden"], "⚡  ", "");
    assert!(!rt.join("noren-pick-42.hidden").exists());
    assert!(names_listed(&env, &[]).contains(&"bg".to_string()));
}

#[test]
fn blacklisted_entries_explain_where_to_unhide_them() {
    let env = setup();
    write_config(&env, "blacklist = [\"bg\"]\n");
    let out = pick(&env, &["hide", &row("zellij", "bg", "live")], "⚡  ", "");
    assert!(out.contains("hidden by blacklist in config.toml"), "{out}");
    // The toggle was undone: nothing recorded.
    let f = env.home.join(".local/state/noren/hidden");
    assert!(fs::read_to_string(&f).unwrap_or_default().is_empty());
}

#[test]
fn separator_and_snapshot_rows_cannot_be_hidden() {
    let env = setup();
    let out = pick(&env, &["hide", "sep\t\t\tsep\t╌"], "⚡  ", "");
    assert!(out.starts_with("change-ghost(✗"), "{out}");
}

#[test]
fn header_event_wraps_to_fzf_columns() {
    let env = setup();
    let (out, _, _) = env.run_env(&["pick", "header"], &[("FZF_COLUMNS", "40")]);
    assert!(out.lines().count() >= 4, "{out}");
    assert!(out.contains("M-h") && out.contains("^h"), "{out}");
}

// ---- rename / mkdir / path search ------------------------------------------

#[test]
fn rename_prompts_inline_and_enter_applies() {
    let env = setup();
    let out = pick(&env, &["rename", &row("zellij", "bg", "live")], "▢  ", "");
    let prompt = "✎ rename bg → ";
    assert!(out.contains(&format!("change-prompt({prompt})")), "{out}");
    assert!(out.contains("change-query(bg)"), "{out}");
    assert!(out.contains("disable-search"), "{out}");
    let out = pick(
        &env,
        &["enter", &row("zellij", "bg", "live")],
        prompt,
        "work",
    );
    assert!(out.contains("✓ renamed bg → work"), "{out}");
    assert!(out.contains("enable-search"), "{out}");
    assert!(out.contains("change-prompt(▢  )"), "{out}");
    assert!(
        env.zellij_log()
            .iter()
            .any(|l| l == "action rename-session work"),
        "{:?}",
        env.zellij_log()
    );
}

#[test]
fn rename_refuses_exited_sessions_and_taken_names_and_esc_cancels() {
    let env = setup();
    let out = pick(
        &env,
        &["rename", &row("zellij", "dead", "exited")],
        "⚡  ",
        "",
    );
    assert!(out.contains("only running sessions"), "{out}");
    pick(&env, &["rename", &row("zellij", "bg", "live")], "⚡  ", "");
    let prompt = "✎ rename bg → ";
    let out = pick(&env, &["enter", ""], prompt, "att");
    assert!(out.contains("already exists"), "{out}");
    pick(&env, &["rename", &row("zellij", "bg", "live")], "⚡  ", "");
    let out = pick(&env, &["esc"], prompt, "zzz");
    assert!(out.contains("rename cancelled"), "{out}");
    assert!(
        !env.zellij_log()
            .iter()
            .any(|l| l.contains("rename-session"))
    );
}

#[test]
fn enter_on_a_new_path_offers_mkdir() {
    let env = setup();
    let target = env.home.join("new/proj");
    let q = target.to_str().unwrap();
    let out = pick(&env, &["enter", ""], "⚡  ", q);
    assert!(out.contains("⚠ create"), "{out}");
    assert!(out.contains("and a session there? y/N"), "{out}");
    let prompt = out
        .split("change-prompt(")
        .nth(1)
        .and_then(|r| r.split(")+").next())
        .unwrap()
        .to_string();
    let out = pick(&env, &["answer"], &prompt, "y");
    assert!(
        out.contains(&format!("print(__noren_mkdir__\t{q})")),
        "{out}"
    );
    assert!(!target.exists(), "created only after fzf exits");
    // A plain name with no match still creates a session from the query.
    assert_eq!(
        pick(&env, &["enter", ""], "⚡  ", "newname"),
        "accept-or-print-query"
    );
    // An existing path is connected as typed, whatever row is under the
    // cursor (fuzzy matches of a typed path are coincidental)...
    assert_eq!(
        pick(
            &env,
            &["enter", "zoxide\tetc\t/etc\tdir\tetc"],
            "⚡  ",
            "/tmp"
        ),
        "print-query"
    );
    // ...unless that row IS the path.
    assert_eq!(
        pick(
            &env,
            &["enter", "zoxide\ttmp\t/tmp\tdir\ttmp"],
            "⚡  ",
            "/tmp"
        ),
        "accept"
    );
    // A stale match under the cursor doesn't stop the mkdir offer.
    let out = pick(&env, &["enter", "zoxide\tetc\t/etc\tdir\tetc"], "⚡  ", q);
    assert!(out.contains("⚠ create"), "{out}");
}

#[test]
fn alt_p_toggles_path_search() {
    let env = setup();
    let rt = env.home.join("run");
    fs::create_dir_all(&rt).unwrap();
    let run = |nth: &str| {
        env.run_env(
            &["pick", "paths"],
            &[("FZF_NTH", nth), ("XDG_RUNTIME_DIR", rt.to_str().unwrap())],
        )
        .0
    };
    assert!(run("1").starts_with("change-nth(1..)+toggle-hscroll"));
    assert!(run("1..").starts_with("change-nth(1)+toggle-hscroll"));
}

#[test]
fn list_marks_the_last_session() {
    let env = setup();
    let state = env.home.join(".local/state/noren");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("previous"), "bg").unwrap();
    let (out, _, _) = env.run(&["list", "zellij"]);
    let bg = out.lines().find(|l| l.starts_with("zellij\tbg\t")).unwrap();
    assert!(bg.contains("bg \x1b[2m↩\x1b[0m"), "{bg:?}");
    assert!(
        !out.lines()
            .find(|l| l.starts_with("zellij\tatt\t"))
            .unwrap()
            .contains('↩')
    );
}
