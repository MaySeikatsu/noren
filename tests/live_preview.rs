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
    let (out, err, ok) = env.run_env(&["preview", "zellij\tlv\t\tlive\t▢ lv"], &DIMS);
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
