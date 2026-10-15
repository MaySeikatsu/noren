//! End-to-end tests for row data, previews and on-disk state carried across
//! renames/switches. Same harness as tests/cli.rs: stubbed `zellij`/`zoxide`
//! on PATH, isolated HOME.

use std::fs;
use std::process::Command;

mod common;
use common::*;

fn write_config(env: &Env, body: &str) {
    let dir = env.home.join(".config/noren");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("config.toml"), body).unwrap();
}

/// Run noren with HOME as the cwd too, so a command-substitution escape
/// (`$(touch pwned)`) would land inside the isolated HOME, not the repo.
fn run_in_home(env: &Env, args: &[&str]) -> (String, bool) {
    let path = format!(
        "{}:{}",
        env.bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(BIN)
        .args(args)
        .current_dir(&env.home)
        .env("HOME", &env.home)
        .env("PATH", path)
        .env_remove("ZELLIJ")
        .env_remove("ZELLIJ_SESSION_NAME")
        .output()
        .expect("run noren");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

#[test]
fn preview_command_gets_the_path_as_one_quoted_argument() {
    let env = Env::new();
    // Space, single quote, and two command-substitution payloads (one
    // relative to the cwd, one absolute under the isolated HOME). The dir
    // need not exist — only the eza fallback checks for that.
    let path = "/nowhere/a b'c$(touch pwned)/x$(touch $HOME/pwned2)";
    let row = format!("zoxide\tx\t{path}\tdir\tx");

    // Bare, single-quoted and double-quoted placeholders all work.
    for placeholder in ["{}", "'{}'", "\\\"{}\\\""] {
        write_config(
            &env,
            &format!(
                "[[wildcard]]\npattern = \"/**\"\npreview_command = \"printf '<%s>\\\\n' {placeholder}\"\n"
            ),
        );
        let (out, ok) = run_in_home(&env, &["preview", &row]);
        assert!(ok);
        assert_eq!(out, format!("<{path}>\n\n"), "placeholder {placeholder}");
    }
    assert!(!env.home.join("pwned").exists());
    assert!(!env.home.join("pwned2").exists());

    // The raw path is also exported for commands that prefer the env.
    write_config(
        &env,
        "[[wildcard]]\npattern = \"/**\"\npreview_command = 'printf \"%s\" \"$NOREN_PATH\"'\n",
    );
    let (out, _) = run_in_home(&env, &["preview", &row]);
    assert_eq!(out, format!("{path}\n"));
}

#[test]
fn config_preview_command_is_quoted_too() {
    let env = Env::new();
    write_config(
        &env,
        "[[session]]\nname = \"proj\"\npath = \"/tmp/my proj\"\npreview_command = \"printf '<%s>' {}\"\n",
    );
    let (out, ok) = run_in_home(
        &env,
        &["preview", "config\tproj\t/tmp/my proj\tconfig\tproj"],
    );
    assert!(ok);
    assert_eq!(out, "</tmp/my proj>\n");
}

fn install_layout(env: &Env, session: &str, src: &str) {
    let info = env
        .home
        .join(".cache/zellij/contract_version_1/session_info")
        .join(session);
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("session-layout.kdl"), src).unwrap();
}

#[test]
fn session_preview_starts_with_focus_summary_then_unchanged_render() {
    let env = Env::new();
    let fixture = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/session-layout.kdl"
    ))
    .unwrap();
    install_layout(&env, "mysess", &fixture);
    write_config(&env, "preview_mode = \"text\"\n");

    let (out, _, ok) = env.run(&["preview", "zellij\tmysess\t\tlive\t▢ mysess"]);
    assert!(ok);
    let (first, rest) = out.split_once('\n').unwrap();
    assert_eq!(
        first,
        "\x1b[1m▸ Tab #1 · claude · /home/maike/.config/nixos\x1b[0m"
    );
    // The golden text render follows byte-for-byte.
    assert!(rest.starts_with("cwd: /home/maike/.config/nixos\n\nTab 1: Tab #1  *\n"));

    // Truncated to the preview width.
    let (out, _, _) = env.run_env(
        &["preview", "zellij\tmysess\t\tlive\t▢ mysess"],
        &[("FZF_PREVIEW_COLUMNS", "24"), ("FZF_PREVIEW_LINES", "20")],
    );
    let first = out.lines().next().unwrap();
    assert_eq!(first, "\x1b[1m▸ Tab #1 · claude · /ho…\x1b[0m");
}

#[test]
fn session_preview_resolves_relative_pane_cwd_and_abbreviates_home() {
    let env = Env::new();
    let home = env.home.display();
    install_layout(
        &env,
        "s",
        &format!(
            "layout {{\n    cwd \"{home}\"\n    tab name=\"code\" focus=true {{\n        pane command=\"/nix/store/x/bin/hx\" cwd=\"Projects/noren\" focus=true\n    }}\n}}\n"
        ),
    );
    let (out, _, _) = env.run(&["preview", "zellij\ts\t\texited\t⊗ s"]);
    assert_eq!(
        out.lines().next().unwrap(),
        "\x1b[1m▸ code · hx · ~/Projects/noren\x1b[0m"
    );
}

#[test]
fn session_preview_without_focus_markers_has_no_summary() {
    let env = Env::new();
    install_layout(
        &env,
        "s",
        "layout {\n    cwd \"/x\"\n    tab name=\"t\" {\n        pane\n    }\n}\n",
    );
    let (out, _, _) = env.run(&["preview", "zellij\ts\t\tlive\t▢ s"]);
    assert_eq!(out, "cwd: /x\n\nTab 1: t\n  └─ (shell)\n");
}
