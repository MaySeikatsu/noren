//! Interactive fzf picker. Keybinds, prompts, and the header line are a
//! muscle-memory contract with zjp2 — do not change them (handover §3.4).

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;

use crate::config::Config;
use crate::connect::session_connect;
use crate::sources::{Selection, collect, rows_to_tsv};

const HEADER: &str =
    "^t sessions  ^z dirs  ^c config  ^a all  ^p pin  ^d kill  ^alt-d delete  ^r reload";

/// The binary fzf binds re-exec. current_exe keeps the picker working when
/// noren isn't on PATH (cargo target dir, nix result/).
fn self_exe() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "noren".to_string())
}

pub fn picker(cfg: &Config) -> anyhow::Result<()> {
    let rows = collect(cfg, Selection::all());
    let tsv = rows_to_tsv(&rows);
    if tsv.is_empty() {
        eprintln!("noren: no sessions, no zoxide dirs, no config entries");
        return Ok(());
    }

    // Every reload/pin/kill bind re-execs this binary; startup is instant.
    let exe = self_exe();
    let mut child = Command::new("fzf")
        .args([
            "--ansi",
            "--no-sort",
            "--delimiter",
            "\t",
            "--with-nth",
            "5",
            "--prompt",
            "⚡  ",
            "--header",
            HEADER,
            "--bind",
            &format!("ctrl-t:reload({exe} list zellij)+change-prompt(▢  )"),
            "--bind",
            &format!("ctrl-z:reload({exe} list zoxide)+change-prompt(📁  )"),
            "--bind",
            &format!("ctrl-c:reload({exe} list config)+change-prompt(⚙  )"),
            "--bind",
            &format!("ctrl-a:reload({exe} list all)+change-prompt(⚡  )"),
            "--bind",
            &format!("ctrl-r:reload({exe} list all)"),
            "--bind",
            &format!("ctrl-p:execute-silent({exe} pin-row {{}})+reload({exe} list all)"),
            "--bind",
            &format!("ctrl-d:execute-silent({exe} kill {{2}})+reload({exe} list all)"),
            "--bind",
            &format!("ctrl-alt-d:execute({exe} delete {{2}})+reload({exe} list all)"),
            "--preview",
            &format!("{exe} preview {{}}"),
            "--preview-window",
            "right,60%",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("noren: failed to spawn fzf (is it on PATH?)")?;

    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(format!("{tsv}\n").as_bytes())
        .ok();
    let out = child.wait_with_output()?;

    // Non-zero = ESC / ctrl-c abort; just close the picker.
    if !out.status.success() {
        return Ok(());
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if line.is_empty() {
        return Ok(());
    }

    // Prefer the path when present (routes through wildcard resolution).
    let mut parts = line.split('\t');
    let _source = parts.next();
    let name = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let target = if !path.is_empty() && std::path::Path::new(path).exists() {
        path
    } else {
        name
    };
    if target.is_empty() {
        return Ok(());
    }
    session_connect(cfg, target)
}
