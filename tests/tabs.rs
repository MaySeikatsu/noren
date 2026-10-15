//! Tab menu: rows for live/exited sessions, picker events, landing.

use std::fs;

mod common;
use common::*;

/// `lv` runs (tabs notes/code/logs, code active); `dead` is exited.
const ZELLIJ: &str = r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1" in
  list-sessions) printf 'lv [Created 1h ago]\ndead [Created 1h ago] (EXITED - attach to resurrect)\n'; exit 0;;
esac
[ "$1" = action ] || exit 0
case "$2" in
  list-tabs) echo '[{"position":2,"name":"logs","active":false},{"position":0,"name":"notes","active":false},{"position":1,"name":"code","active":true}]';;
  list-panes) echo '[{"id":1,"is_plugin":false,"is_focused":true,"is_floating":false,"tab_position":1,"title":"hx","pane_cwd":"/tmp"}]';;
  list-clients) echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND";;
esac
exit 0"#;

fn setup() -> Env {
    let env = Env::new();
    env.stub("zellij", ZELLIJ);
    env
}

fn pick(env: &Env, args: &[&str], prompt: &str) -> String {
    let rt = env.home.join("run");
    fs::create_dir_all(&rt).unwrap();
    let mut full = vec!["pick"];
    full.extend_from_slice(args);
    let (out, err, ok) = env.run_env(
        &full,
        &[
            ("NOREN_PICKER_ID", "11"),
            ("XDG_RUNTIME_DIR", rt.to_str().unwrap()),
            ("FZF_PROMPT", prompt),
            ("FZF_QUERY", ""),
        ],
    );
    assert!(ok, "{err}");
    out
}

#[test]
fn live_session_tab_rows_in_order_with_active_mark_and_detail() {
    let env = setup();
    let (out, _, ok) = env.run(&["tab-rows", "lv"]);
    assert!(ok);
    let rows: Vec<Vec<&str>> = out.lines().map(|l| l.split('\t').collect()).collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(&rows[0][..4], ["tab", "lv", "1", "tab"]);
    assert!(rows[0][4].starts_with("▤ 1   notes"), "{:?}", rows[0][4]);
    assert!(rows[1][4].starts_with("▤ 2 ● code"), "{:?}", rows[1][4]);
    assert!(rows[1][4].contains("hx · /tmp"), "{:?}", rows[1][4]);
    assert_eq!(rows[2][2], "3");
}

#[test]
fn exited_session_tabs_come_from_the_saved_layout_without_asking_zellij() {
    let env = setup();
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/dead");
    fs::create_dir_all(&info).unwrap();
    fs::write(
        info.join("session-layout.kdl"),
        "layout {\n    tab name=\"a\" {\n    }\n    tab name=\"b\" focus=true {\n    }\n}\n",
    )
    .unwrap();
    let (out, _, _) = env.run(&["tab-rows", "dead"]);
    assert!(out.lines().next().unwrap().contains("▤ 1   a"), "{out}");
    assert!(out.lines().nth(1).unwrap().contains("▤ 2 ● b"), "{out}");
    assert!(out.contains("saved layout"));
    assert!(
        !env.zellij_log().iter().any(|l| l.starts_with("action")),
        "{:?}",
        env.zellij_log()
    );
}

#[test]
fn alt_t_opens_the_menu_enter_hands_off_esc_returns() {
    let env = setup();
    let row = "zellij\tlv\t\tlive\t▢ lv";
    let out = pick(&env, &["tabs", row], "▢  ");
    assert!(out.contains("change-prompt(▤ lv  )"), "{out}");
    assert!(out.contains(" tab-rows 'lv')"), "{out}");
    // ⏎ on a tab: print the hand-off marker and leave fzf.
    let out = pick(&env, &["enter", "tab\tlv\t3\ttab\t▤ 3 logs"], "▤ lv  ");
    assert_eq!(out, "print(__noren_tab__\tlv\t3)+accept");
    // No match inside the menu creates nothing.
    assert_eq!(pick(&env, &["enter", ""], "▤ lv  "), "ignore");
    // esc goes back to the view it came from.
    let out = pick(&env, &["esc"], "▤ lv  ");
    assert!(out.contains("change-prompt(▢  )"), "{out}");
    // Folders have no tabs.
    let out = pick(&env, &["tabs", "zoxide\ttmp\t/tmp\tdir\ttmp"], "⚡  ");
    assert!(out.starts_with("change-ghost(✗"), "{out}");
}

#[test]
fn go_to_tab_helper_waits_for_the_new_client() {
    // list-clients reports one client only after the "attach" happened
    // (a marker file), like a real attach racing the helper.
    let env = Env::new();
    env.stub(
        "zellij",
        r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1" in
  list-sessions) echo 'lv [Created 1h ago]'; exit 0;;
esac
if [ "$2" = list-clients ]; then
  echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
  [ -f "$HOME/attached" ] && echo "1 terminal_0 N/A"
fi
exit 0"#,
    );
    let home = env.home.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        fs::write(home.join("attached"), "").unwrap();
    });
    let (_, _, ok) = env.run(&["goto-tab", "lv", "3", "0"]);
    assert!(ok);
    let log = env.zellij_log();
    assert_eq!(
        log.iter()
            .filter(|l| l.starts_with("action go-to-tab"))
            .count(),
        1,
        "{log:?}"
    );
    assert!(log.iter().any(|l| l == "action go-to-tab 3"), "{log:?}");
}
