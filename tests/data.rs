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
