//! End-to-end tests against the built binary, with stubbed `zellij`/`zoxide`
//! on PATH and an isolated HOME.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_zjp3");

/// Isolated HOME + stub-bin dir prepended to PATH.
struct Env {
    _tmp: tempfile::TempDir,
    home: std::path::PathBuf,
    bin: std::path::PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        Env {
            home,
            bin,
            _tmp: tmp,
        }
    }

    fn stub(&self, name: &str, script: &str) {
        let p = self.bin.join(name);
        fs::write(&p, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run(&self, args: &[&str]) -> (String, String, bool) {
        self.run_env(args, &[])
    }

    fn run_env(&self, args: &[&str], extra_env: &[(&str, &str)]) -> (String, String, bool) {
        let path = format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env("HOME", &self.home)
            .env("PATH", path)
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run zjp3");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.success(),
        )
    }

    /// Stub zellij that appends its argv to $HOME/zellij-args.log.
    fn stub_zellij_logging(&self) {
        self.stub("zellij", "echo \"$@\" >> \"$HOME/zellij-args.log\"\nexit 0");
    }

    fn zellij_log(&self) -> Vec<String> {
        fs::read_to_string(self.home.join("zellij-args.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

#[test]
fn list_zoxide_uses_stubbed_binary() {
    let env = Env::new();
    env.stub("zoxide", "echo /tmp/alpha\necho /tmp/beta");
    env.stub("zellij", "exit 0");
    let (out, _, ok) = env.run(&["list", "zoxide"]);
    assert!(ok);
    assert_eq!(
        out,
        "zoxide\talpha\t/tmp/alpha\tdir\t📁 alpha\nzoxide\tbeta\t/tmp/beta\tdir\t📁 beta\n"
    );
}

#[test]
fn list_zellij_parses_states_and_pins() {
    let env = Env::new();
    env.stub(
        "zellij",
        r#"echo "work [Created 1h ago]"
echo "old [Created 2days ago] (EXITED - attach to resurrect)""#,
    );
    env.stub("zoxide", "exit 0");
    fs::create_dir_all(env.home.join(".local/state/zellij")).unwrap();
    fs::write(env.home.join(".local/state/zellij/pinned"), "work\n").unwrap();

    let (out, _, ok) = env.run(&["list", "zellij"]);
    assert!(ok);
    // First row sits next to the prompt: pinned, then live, then exited.
    let lines: Vec<&str> = out.trim_end().split('\n').collect();
    assert_eq!(lines[0], "zellij\twork\t\tlive\t📌 ▢ work");
    assert_eq!(lines[1], "zellij\told\t\texited\t⊗ old");
}

#[test]
fn list_zellij_marks_attached_sessions() {
    let env = Env::new();
    // `att` has a client attached, `bg` runs in the background. The stub only
    // answers list-clients with the real header for `att`.
    env.stub(
        "zellij",
        r#"case "$1" in
  list-sessions)
    echo "att [Created 1h ago]"
    echo "bg [Created 1h ago]";;
  action)
    echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
    if [ "$ZELLIJ_SESSION_NAME" = "att" ]; then
      echo "1         terminal_0     N/A"
    fi;;
esac"#,
    );
    env.stub("zoxide", "exit 0");
    let (out, _, ok) = env.run(&["list", "zellij"]);
    assert!(ok);
    let lines: Vec<&str> = out.trim_end().split('\n').collect();
    assert_eq!(lines[0], "zellij\tatt\t\tlive\t▣ att");
    assert_eq!(lines[1], "zellij\tbg\t\tlive\t▢ bg");
}

#[test]
fn list_all_merges_sorts_dedupes_and_blacklists() {
    let env = Env::new();
    env.stub(
        "zellij",
        "echo \"alpha [Created 1h ago]\"\necho \"junk [Created 1h ago]\"",
    );
    env.stub("zoxide", "echo /tmp/alpha");
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "blacklist = [\"junk\"]\n",
    )
    .unwrap();

    let (out, _, ok) = env.run(&["list", "all"]);
    assert!(ok);
    // Sessions come first now, so dedupe keeps the zellij "alpha" (the live
    // row beats the dir row); blacklist hides "junk". Only one area is left,
    // so no separator line appears.
    assert_eq!(out, "zellij\talpha\t\tlive\t▢ alpha\n");

    let (out, _, _) = env.run(&["list", "blacklist"]);
    assert_eq!(out, "zellij\tjunk\t\tlive\t▢ junk\n");
}

#[test]
fn list_all_draws_separator_between_sessions_and_folders() {
    let env = Env::new();
    env.stub("zellij", "echo \"work [Created 1h ago]\"");
    env.stub("zoxide", "echo /tmp/somedir");

    let (out, _, ok) = env.run(&["list", "all"]);
    assert!(ok);
    let lines: Vec<&str> = out.trim_end().split('\n').collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("zellij\twork"));
    assert!(lines[1].starts_with("sep\t\t\tsep\t"), "got: {}", lines[1]);
    assert!(lines[1].contains("╌"));
    assert!(lines[2].starts_with("zoxide\tsomedir"));

    // Configurable: separator = false drops the line.
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "separator = false\n",
    )
    .unwrap();
    let (out, _, _) = env.run(&["list", "all"]);
    assert_eq!(out.trim_end().split('\n').count(), 2);
}

#[test]
fn pin_row_pins_dirs_separately_and_sorts_them_first_in_dir_group() {
    let env = Env::new();
    env.stub("zellij", "echo \"work [Created 1h ago]\"");
    env.stub("zoxide", "echo /tmp/aaa\necho /tmp/bbb");

    // Pin the second dir via the picker-internal row toggle.
    let (out, _, ok) = env.run(&["pin-row", "zoxide\tbbb\t/tmp/bbb\tdir\t📁 bbb"]);
    assert!(ok);
    assert_eq!(out, "/tmp/bbb: pinned\n");
    // Dir pins land in zjp3's own file, NOT the shared session pin file.
    let dirs = fs::read_to_string(env.home.join(".local/state/zjp/pinned-dirs")).unwrap();
    assert_eq!(dirs, "/tmp/bbb\n");
    assert!(!env.home.join(".local/state/zellij/pinned").exists());

    // Pinned dir floats to the front of the dir group, above the separator.
    let (out, _, _) = env.run(&["list", "all"]);
    let lines: Vec<&str> = out.trim_end().split('\n').collect();
    assert!(lines[0].starts_with("zellij\twork"));
    assert!(lines[1].starts_with("sep\t"));
    assert_eq!(lines[2], "zoxide\tbbb\t/tmp/bbb\tdir\t📌 📁 bbb");
    assert!(lines[3].starts_with("zoxide\taaa"));

    // pin-row on a session row toggles the shared file, same as `zjp3 pin`.
    let (out, _, _) = env.run(&["pin-row", "zellij\twork\t\tlive\t▢ work"]);
    assert_eq!(out, "work: pinned\n");
    let pins = fs::read_to_string(env.home.join(".local/state/zellij/pinned")).unwrap();
    assert_eq!(pins, "work\n");

    // Toggle the dir pin off again.
    let (out, _, _) = env.run(&["pin-row", "zoxide\tbbb\t/tmp/bbb\tdir\t📌 📁 bbb"]);
    assert_eq!(out, "/tmp/bbb: unpinned\n");
}

#[test]
fn pinned_dirs_join_session_pins_when_configured() {
    let env = Env::new();
    env.stub(
        "zellij",
        "echo \"pinned-sess [Created 1h ago]\"\necho \"live-sess [Created 1h ago]\"",
    );
    env.stub("zoxide", "echo /tmp/aaa\necho /tmp/bbb");
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "pinned_dirs_with_sessions = true\n",
    )
    .unwrap();
    fs::create_dir_all(env.home.join(".local/state/zellij")).unwrap();
    fs::write(env.home.join(".local/state/zellij/pinned"), "pinned-sess\n").unwrap();
    fs::create_dir_all(env.home.join(".local/state/zjp")).unwrap();
    fs::write(env.home.join(".local/state/zjp/pinned-dirs"), "/tmp/bbb\n").unwrap();

    let (out, _, ok) = env.run(&["list", "all"]);
    assert!(ok);
    let lines: Vec<&str> = out.trim_end().split('\n').collect();
    // Pinned dir slots in right after the pinned session, before live ones;
    // the separator moves above the whole session area.
    assert!(lines[0].starts_with("zellij\tpinned-sess"));
    assert_eq!(lines[1], "zoxide\tbbb\t/tmp/bbb\tdir\t📌 📁 bbb");
    assert!(lines[2].starts_with("zellij\tlive-sess"));
    assert!(lines[3].starts_with("sep\t"));
    assert!(lines[4].starts_with("zoxide\taaa"));
}

#[test]
fn preview_visual_mode_draws_pane_boxes() {
    let env = Env::new();
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/mysess");
    fs::create_dir_all(&info).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session-layout.kdl");
    fs::copy(fixture, info.join("session-layout.kdl")).unwrap();

    // fzf exports the preview size; visual mode is the default under fzf.
    let (out, _, ok) = env.run_env(
        &["preview", "zellij\tmysess\t\tlive\t▢ mysess"],
        &[("FZF_PREVIEW_COLUMNS", "60"), ("FZF_PREVIEW_LINES", "20")],
    );
    assert!(ok);
    assert!(out.contains("cwd: /home/maike/.config/nixos"), "got: {out}");
    // The focused pane gets a heavy border; both commands are labeled.
    assert!(out.contains('┏'), "got: {out}");
    assert!(out.contains('┌'), "got: {out}");
    assert!(out.contains("claude *"), "got: {out}");
    assert!(out.contains("nvim"), "got: {out}");
    assert!(out.contains("(+ 1 floating)"), "got: {out}");

    // preview_mode = "text" opts back into the tree render.
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "preview_mode = \"text\"\n",
    )
    .unwrap();
    let (out, _, _) = env.run_env(
        &["preview", "zellij\tmysess\t\tlive\t▢ mysess"],
        &[("FZF_PREVIEW_COLUMNS", "60"), ("FZF_PREVIEW_LINES", "20")],
    );
    assert!(out.contains("├─ V-split"), "got: {out}");
    assert!(!out.contains('┏'), "got: {out}");
}

#[test]
fn preview_of_separator_row_is_blank() {
    let env = Env::new();
    let (out, _, ok) = env.run(&["preview", "sep\t\t\tsep\t╌╌╌"]);
    assert!(ok);
    assert_eq!(out, "\n");
}

#[test]
fn preview_renders_saved_layout_natively() {
    let env = Env::new();
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/mysess");
    fs::create_dir_all(&info).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/session-layout.kdl");
    fs::copy(fixture, info.join("session-layout.kdl")).unwrap();

    let (out, _, ok) = env.run(&["preview", "zellij\tmysess\t\tlive\t▢ mysess"]);
    assert!(ok);
    assert!(out.contains("cwd: /home/maike/.config/nixos"), "got: {out}");
    assert!(out.contains("Tab 1: Tab #1  *"), "got: {out}");
    assert!(out.contains("— claude [50%] *"), "got: {out}");
    assert!(out.contains("(+ 1 floating)"), "got: {out}");
}

#[test]
fn preview_missing_layout_is_friendly() {
    let env = Env::new();
    let (out, _, ok) = env.run(&["preview", "zellij\tghost\t\texited\t⊗ ghost"]);
    assert!(ok);
    assert_eq!(out, "(no saved layout)\n");
}

#[test]
fn pin_toggles_shared_pin_file() {
    let env = Env::new();
    let (out, _, ok) = env.run(&["pin", "foo"]);
    assert!(ok);
    assert_eq!(out, "foo: pinned\n");
    let pinned = fs::read_to_string(env.home.join(".local/state/zellij/pinned")).unwrap();
    // Trailing newline is part of the contract (zpin appends with echo >>).
    assert_eq!(pinned, "foo\n");

    let (out, _, _) = env.run(&["pin", "foo"]);
    assert_eq!(out, "foo: unpinned\n");
    let pinned = fs::read_to_string(env.home.join(".local/state/zellij/pinned")).unwrap();
    assert_eq!(pinned, "");
}

#[test]
fn pin_without_name_outside_zellij_fails() {
    let env = Env::new();
    let (_, err, ok) = env.run(&["pin"]);
    assert!(!ok);
    assert!(err.contains("no name given"));
}

#[test]
fn name_for_and_resolve_env_format() {
    let env = Env::new();
    env.stub("zellij", "exit 0");
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        r#"
[[session]]
name = "proj"
path = "/tmp"
layout = "ide-git"
startup_command = "hx ."
"#,
    )
    .unwrap();

    let (out, _, ok) = env.run(&["name-for", "/tmp"]);
    assert!(ok);
    assert_eq!(out, "proj\n");

    // $HOME special case (parity with zellij-autostart)
    let (out, _, _) = env.run(&["name-for", &env.home.to_string_lossy()]);
    assert_eq!(out, "home\n");

    let (out, _, _) = env.run(&["resolve", "/tmp"]);
    assert_eq!(
        out,
        "NAME='proj'\nSESSION_PATH='/tmp'\nLAYOUT='ide-git'\nSTARTUP='hx .'\n"
    );

    let (out, _, _) = env.run(&["resolve", "/tmp", "--format", "json"]);
    assert!(out.contains("\"name\":\"proj\""), "got: {out}");
    assert!(out.contains("\"layout\":\"ide-git\""), "got: {out}");
}

#[test]
fn last_without_state_fails_cleanly() {
    let env = Env::new();
    let (_, err, ok) = env.run(&["last"]);
    assert!(!ok);
    assert!(err.contains("no previous session recorded yet"));
}

#[test]
fn malformed_config_warns_and_uses_defaults() {
    let env = Env::new();
    env.stub("zellij", "exit 0");
    env.stub("zoxide", "echo /tmp/x");
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "sort_order = [oops",
    )
    .unwrap();
    let (out, err, ok) = env.run(&["list", "zoxide"]);
    assert!(ok);
    assert!(err.contains("warning"));
    assert!(out.contains("/tmp/x"));
}

#[test]
fn connect_outside_zellij_execs_attach_create() {
    let env = Env::new();
    env.stub_zellij_logging();
    // Unknown subcommand falls through to connect (sesh shorthand).
    let (_, _, ok) = env.run(&["ghost-name"]);
    assert!(ok);
    let log = env.zellij_log();
    assert_eq!(log.last().unwrap(), "attach --create ghost-name");
    // last-session state is recorded on connect
    let last = fs::read_to_string(env.home.join(".local/state/zjp/last")).unwrap();
    assert_eq!(last, "ghost-name");
}

#[test]
fn connect_outside_with_layout_uses_new_session_with_layout() {
    let env = Env::new();
    env.stub_zellij_logging();
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "[[session]]\nname = \"proj\"\nlayout = \"ide\"\n",
    )
    .unwrap();
    let layouts = env.home.join(".config/zellij/layouts");
    fs::create_dir_all(&layouts).unwrap();
    fs::write(layouts.join("ide.kdl"), "layout {\n}\n").unwrap();

    let (_, _, ok) = env.run(&["connect", "proj"]);
    assert!(ok);
    let expected = format!(
        "--session proj --new-session-with-layout {}",
        layouts.join("ide.kdl").display()
    );
    assert_eq!(env.zellij_log().last().unwrap(), &expected);
}

#[test]
fn connect_inside_zellij_creates_background_and_pipes_switch() {
    let env = Env::new();
    env.stub_zellij_logging();
    let (_, _, ok) = env.run_env(&["connect", "ghost"], &[("ZELLIJ", "1")]);
    assert!(ok);
    let log = env.zellij_log();
    assert!(
        log.iter().any(|l| l == "attach --create-background ghost"),
        "log: {log:?}"
    );
    let switch = format!(
        "pipe --plugin file:{}/.config/zellij/plugins/zellij-switch.wasm -- --session ghost",
        env.home.display()
    );
    assert_eq!(log.last().unwrap(), &switch);
}

#[test]
fn help_lists_all_subcommands() {
    let env = Env::new();
    let (out, _, ok) = env.run(&["help"]);
    assert!(ok);
    for sub in [
        "list", "connect", "last", "root", "kill", "delete", "discard", "mkdir", "clone", "pin",
        "rename", "snapshot", "snapshots", "restore", "window", "preview", "picker", "name-for",
        "resolve",
    ] {
        assert!(out.contains(sub), "help missing `{sub}`:\n{out}");
    }
}

#[test]
fn discard_inside_switches_to_previous_then_kills() {
    let env = Env::new();
    // Stub answers list-sessions with two live sessions and logs everything.
    env.stub(
        "zellij",
        r#"echo "$@" >> "$HOME/zellij-args.log"
if [ "$1" = "list-sessions" ]; then
  echo "cur [Created 1h ago]"
  echo "other [Created 2h ago]"
fi"#,
    );
    fs::create_dir_all(env.home.join(".local/state/zjp")).unwrap();
    fs::write(env.home.join(".local/state/zjp/previous"), "other\n").unwrap();

    let (out, _, ok) = env.run_env(
        &["discard"],
        &[("ZELLIJ", "1"), ("ZELLIJ_SESSION_NAME", "cur")],
    );
    assert!(ok, "discard failed: {out}");
    assert_eq!(out, "discarded: cur\n");
    let log = env.zellij_log();
    let switch = format!(
        "pipe --plugin file:{}/.config/zellij/plugins/zellij-switch.wasm -- --session other",
        env.home.display()
    );
    assert!(log.iter().any(|l| l == &switch), "log: {log:?}");
    assert_eq!(log.last().unwrap(), "kill-session cur");
    // The switch happens BEFORE the kill.
    let si = log.iter().position(|l| l == &switch).unwrap();
    let ki = log.iter().position(|l| l == "kill-session cur").unwrap();
    assert!(si < ki);
}

#[test]
fn snapshot_dumps_live_layout_and_prunes_to_keep() {
    let env = Env::new();
    // dump-layout output changes each call (counter file) so snapshots differ;
    // list-clients (attached indicator) answers separately and must not bump it.
    env.stub(
        "zellij",
        r#"case "$1" in
  list-sessions) echo "foo [Created 1h ago]";;
  action)
    if [ "$2" = "dump-layout" ]; then
      n=$(cat "$HOME/n" 2>/dev/null || echo 0)
      echo $((n + 1)) > "$HOME/n"
      echo "layout { cwd \"/tmp\" } // v$n"
    else
      echo "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND"
    fi;;
esac"#,
    );
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "snapshot_keep = 2\n",
    )
    .unwrap();

    for _ in 0..3 {
        let (out, err, ok) = env.run(&["snapshot", "foo"]);
        assert!(ok, "snapshot failed: {out} {err}");
        assert!(out.starts_with("snapshot: "), "got: {out}");
    }
    let dir = env.home.join(".local/state/zjp/snapshots/foo");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    // Pruned to snapshot_keep = 2; the newest content (v2) survived.
    assert_eq!(files.len(), 2, "files: {files:?}");
    let bodies: Vec<String> = files
        .iter()
        .map(|f| fs::read_to_string(f).unwrap())
        .collect();
    assert!(bodies.iter().any(|b| b.contains("// v2")), "{bodies:?}");
    assert!(!bodies.iter().any(|b| b.contains("// v0")), "{bodies:?}");

    // `snapshots` lists them newest first, 1-based.
    let (out, _, ok) = env.run(&["snapshots", "foo"]);
    assert!(ok);
    assert!(out.starts_with("1: "), "got: {out}");
    assert_eq!(out.lines().count(), 2);
}

#[test]
fn snapshot_of_exited_session_uses_resurrection_cache() {
    let env = Env::new();
    env.stub("zellij", "exit 0"); // no live sessions
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info/ghost");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("session-layout.kdl"), "layout { }\n").unwrap();

    let (out, _, ok) = env.run(&["snapshot", "ghost"]);
    assert!(ok, "got: {out}");
    let dir = env.home.join(".local/state/zjp/snapshots/ghost");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    // Identical content doesn't pile up a duplicate snapshot.
    let (_, _, ok) = env.run(&["snapshot", "ghost"]);
    assert!(ok);
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn restore_outside_execs_new_session_with_snapshot_layout() {
    let env = Env::new();
    env.stub_zellij_logging();
    let dir = env.home.join(".local/state/zjp/snapshots/foo");
    fs::create_dir_all(&dir).unwrap();
    let snap = dir.join("20260713-090000.kdl");
    fs::write(&snap, "layout { }\n").unwrap();

    let (_, _, ok) = env.run(&["restore", "foo", "--force"]);
    assert!(ok);
    let expected = format!("--session foo --new-session-with-layout {}", snap.display());
    assert_eq!(env.zellij_log().last().unwrap(), &expected);
}

#[test]
fn connect_auto_snapshots_pinned_sessions_when_enabled() {
    let env = Env::new();
    env.stub(
        "zellij",
        r#"echo "$@" >> "$HOME/zellij-args.log"
case "$1" in
  list-sessions) echo "foo [Created 1h ago]";;
  action) echo "layout { }";;
esac"#,
    );
    fs::create_dir_all(env.home.join(".config/zjp")).unwrap();
    fs::write(
        env.home.join(".config/zjp/config.toml"),
        "auto_snapshot_pinned = true\n",
    )
    .unwrap();
    fs::create_dir_all(env.home.join(".local/state/zellij")).unwrap();
    fs::write(env.home.join(".local/state/zellij/pinned"), "foo\n").unwrap();

    let (_, _, ok) = env.run_env(&["connect", "foo"], &[("ZELLIJ", "1")]);
    assert!(ok);
    let dir = env.home.join(".local/state/zjp/snapshots/foo");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn completions_cover_all_shells_and_fish_is_dynamic() {
    let env = Env::new();
    let (out, _, ok) = env.run(&["completions", "fish"]);
    assert!(ok);
    assert!(out.contains("complete -c zjp3"), "got: {out}");
    assert!(out.contains("__zjp3_sessions"), "got: {out}");
    assert!(out.contains("__zjp3_targets"), "got: {out}");

    let (out, _, ok) = env.run(&["completions", "zsh"]);
    assert!(ok);
    assert!(out.starts_with("#compdef zjp3"), "got: {out}");

    let (out, _, ok) = env.run(&["completions", "bash"]);
    assert!(ok);
    assert!(out.contains("zjp3"), "got: {out}");

    let (out, _, ok) = env.run(&["completions", "nushell"]);
    assert!(ok);
    assert!(out.contains("export extern"), "got: {out}");

    let (_, err, ok) = env.run(&["completions", "powershell"]);
    assert!(!ok);
    assert!(err.contains("unknown shell"), "got: {err}");
}
