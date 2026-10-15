//! Live preview: running sessions show their current screen; nothing else
//! may ever contact zellij from a preview.

use std::fs;
use std::time::Instant;

mod common;
use common::*;

/// Session `lv` with two tabs ("code" active, focused tiled pane 4 running
/// a shell in ~/proj, a floating picker pane on top), dump with history and
/// trailing blank lines.
const ZELLIJ: &str = r#"echo "$@" >> "$HOME/zellij-args.log"
[ "$1" = list-sessions ] && { echo "lv [Created 1h ago]"; exit 0; }
[ "$1" = action ] || exit 0
case "$2" in
  list-tabs) echo '[{"position":0,"name":"notes","active":false},{"position":1,"name":"code","active":true}]';;
  list-panes) cat <<JSON
[{"id":0,"is_plugin":true,"is_focused":false,"is_floating":false,"tab_position":1,"title":"tab-bar"},
 {"id":2,"is_plugin":false,"is_focused":true,"is_floating":false,"tab_position":0,"title":"vim","pane_cwd":"/tmp"},
 {"id":4,"is_plugin":false,"is_focused":true,"is_floating":false,"tab_position":1,"title":"cargo test","pane_cwd":"$HOME/proj"},
 {"id":5,"is_plugin":false,"is_focused":true,"is_floating":true,"tab_position":1,"title":"noren"}]
JSON
  ;;
  dump-screen) printf 'old line\n$ cargo test\ntest result: ok\n$ \n\n\n';;
esac"#;

fn setup(stub: &str) -> Env {
    let env = Env::new();
    env.stub("zellij", stub);
    env
}

const DIMS: [(&str, &str); 2] = [("FZF_PREVIEW_COLUMNS", "60"), ("FZF_PREVIEW_LINES", "8")];

fn calls(env: &Env) -> Vec<String> {
    env.zellij_log()
}

#[test]
fn live_session_shows_focused_tiled_pane_of_active_tab() {
    let env = setup(ZELLIJ);
    // "live" = the active tab only (the default "tabs" shows all of them).
    let (out, err, ok) = env.run_env(
        &["preview", "--mode", "live", "zellij\tlv\t\tlive\t▢ lv"],
        &DIMS,
    );
    assert!(ok, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines[0],
        "\x1b[32m●\x1b[0m \x1b[1mcode · cargo test · ~/proj\x1b[0m"
    );
    assert!(
        lines[1].contains("\x1b[7m code \x1b[0m"),
        "tab strip: {:?}",
        lines[1]
    );
    // Trailing blank lines dropped; the screen fits the 6 rows left.
    assert_eq!(
        &lines[2..],
        [
            "old line\x1b[0m",
            "$ cargo test\x1b[0m",
            "test result: ok\x1b[0m",
            "$ \x1b[0m"
        ]
    );
    // The tiled pane (4) was dumped, not the floating picker (5).
    assert!(
        calls(&env)
            .iter()
            .any(|c| c == "action dump-screen -p terminal_4 --ansi"),
        "{:?}",
        calls(&env)
    );
}

#[test]
fn session_that_died_since_the_list_is_not_previewed_live() {
    // The row still says live, but zellij no longer lists it: never ask
    // `zellij action` (it would answer for some other running session).
    let env =
        setup("echo \"$@\" >> \"$HOME/zellij-args.log\"; [ \"$1\" = action ] && echo '[]'; exit 0");
    let (out, _, ok) = env.run_env(&["preview", "zellij\tgone\t\tlive\t▢ gone"], &DIMS);
    assert!(ok);
    assert!(!out.contains("\x1b[32m●"), "{out}");
    assert!(
        !calls(&env).iter().any(|c| c.starts_with("action")),
        "{:?}",
        calls(&env)
    );
}

#[test]
fn exited_sessions_never_contact_zellij() {
    let env = setup(ZELLIJ);
    let (_, _, ok) = env.run_env(&["preview", "zellij\tdead\t\texited\t⊗ dead"], &DIMS);
    assert!(ok);
    assert!(calls(&env).is_empty(), "{:?}", calls(&env));
}

#[test]
fn layout_mode_and_layout_configs_never_contact_zellij() {
    let env = setup(ZELLIJ);
    let line = "zellij\tlv\t\tlive\t▢ lv";
    env.run_env(&["preview", "--mode", "layout", line], &DIMS);
    fs::create_dir_all(env.home.join(".config/noren")).unwrap();
    fs::write(
        env.home.join(".config/noren/config.toml"),
        "preview_mode = \"visual\"\n",
    )
    .unwrap();
    env.run_env(&["preview", line], &DIMS);
    assert!(calls(&env).is_empty(), "{:?}", calls(&env));
    // ...while --mode live overrides a layout config.
    let (out, _, _) = env.run_env(&["preview", "--mode", "live", line], &DIMS);
    assert!(out.starts_with("\x1b[32m●"), "{out}");
}

#[test]
fn hung_server_falls_back_to_layout_quickly() {
    let env =
        setup("[ \"$1\" = list-sessions ] && { echo 'lv [Created 1h ago]'; exit 0; }\nsleep 30");
    let start = Instant::now();
    let (out, _, ok) = env.run_env(&["preview", "zellij\tlv\t\tlive\t▢ lv"], &DIMS);
    assert!(ok);
    assert!(start.elapsed().as_secs() < 3, "took {:?}", start.elapsed());
    assert!(!out.contains("\x1b[32m●"), "{out}");
}

#[test]
fn garbage_output_falls_back_to_layout() {
    let env = setup(
        "[ \"$1\" = list-sessions ] && { echo 'lv [Created 1h ago]'; exit 0; }\necho not-json",
    );
    let (out, _, ok) = env.run_env(&["preview", "zellij\tlv\t\tlive\t▢ lv"], &DIMS);
    assert!(ok);
    assert!(!out.contains("\x1b[32m●"), "{out}");
}

#[test]
fn all_tabs_mode_shows_every_tab_side_by_side() {
    let env = setup(ZELLIJ);
    fs::create_dir_all(env.home.join(".config/noren")).unwrap();
    fs::write(
        env.home.join(".config/noren/config.toml"),
        "live_all_tabs = true\n",
    )
    .unwrap();
    let (out, err, ok) = env.run_env(
        &["preview", "zellij\tlv\t\tlive\t▢ lv"],
        &[("FZF_PREVIEW_COLUMNS", "60"), ("FZF_PREVIEW_LINES", "8")],
    );
    assert!(ok, "{err}");
    let plain: Vec<String> = out
        .lines()
        .map(|l| {
            let mut s = String::new();
            let mut esc = false;
            for c in l.chars() {
                match (esc, c) {
                    (false, '\x1b') => esc = true,
                    (true, c) if c.is_ascii_alphabetic() => esc = false,
                    (false, c) => s.push(c),
                    _ => {}
                }
            }
            s
        })
        .collect();
    assert_eq!(plain[0], "● 2 tabs · lv");
    // Two cells, 29 + gutter + 29 = 59 ≤ 60 columns; titles side by side.
    assert!(
        plain[1].starts_with("   notes ") && plain[1].contains("┃ ● code "),
        "{plain:?}"
    );
    // Both tabs' focused panes were dumped (2 = notes, 4 = code).
    let log = env.zellij_log();
    for id in ["terminal_2", "terminal_4"] {
        assert!(log.iter().any(|l| l.contains(id)), "{id}: {log:?}");
    }
    // The active tab's title is in reverse video.
    assert!(
        out.lines().nth(1).unwrap().contains("\x1b[1;7;32m ● code "),
        "{out}"
    );
}

#[test]
fn show_tabs_lists_live_tabs_from_zellij_and_exited_ones_from_the_cache() {
    let env = setup(
        r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1" in
  list-sessions) printf 'lv [Created 1h ago]\ndead [Created 1h ago] (EXITED - attach to resurrect)\n';;
  action) [ "$2" = list-tabs ] && echo '[{"position":1,"name":"code"},{"position":0,"name":"notes"}]';;
esac
exit 0"#,
    );
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/dead");
    fs::create_dir_all(&info).unwrap();
    fs::write(
        info.join("session-layout.kdl"),
        "layout {\n    cwd \"/tmp\"\n    tab name=\"old\" {\n    }\n    tab {\n    }\n}\n",
    )
    .unwrap();
    fs::create_dir_all(env.home.join(".config/noren")).unwrap();
    fs::write(
        env.home.join(".config/noren/config.toml"),
        "show_tabs = \"column\"\nattached_indicator = false\n",
    )
    .unwrap();
    let (out, _, ok) = env.run(&["list", "zellij"]);
    assert!(ok);
    let lv = out.lines().find(|l| l.starts_with("zellij\tlv\t")).unwrap();
    assert!(
        lv.contains("(notes, code)"),
        "tab order by position: {lv:?}"
    );
    let dead = out
        .lines()
        .find(|l| l.starts_with("zellij\tdead\t"))
        .unwrap();
    assert!(dead.contains("(old, Tab #2)"), "{dead:?}");
    // The exited session was never asked.
    let asked: Vec<String> = calls(&env)
        .into_iter()
        .filter(|c| c.starts_with("action"))
        .collect();
    assert_eq!(asked, ["action list-tabs -j"], "{asked:?}");
}
