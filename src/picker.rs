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

pub fn picker(cfg: &Config) -> anyhow::Result<()> {
    let rows = collect(cfg, Selection::all());
    let tsv = rows_to_tsv(&rows);
    if tsv.is_empty() {
        eprintln!("zjp3: no sessions, no zoxide dirs, no config entries");
        return Ok(());
    }

    // Every reload/pin/kill bind re-execs this binary; startup is instant.
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
            "ctrl-t:reload(zjp3 list zellij)+change-prompt(▢  )",
            "--bind",
            "ctrl-z:reload(zjp3 list zoxide)+change-prompt(📁  )",
            "--bind",
            "ctrl-c:reload(zjp3 list config)+change-prompt(⚙  )",
            "--bind",
            "ctrl-a:reload(zjp3 list all)+change-prompt(⚡  )",
            "--bind",
            "ctrl-r:reload(zjp3 list all)",
            "--bind",
            "ctrl-p:execute-silent(zjp3 pin-row {})+reload(zjp3 list all)",
            "--bind",
            "ctrl-d:execute-silent(zjp3 kill {2})+reload(zjp3 list all)",
            "--bind",
            "ctrl-alt-d:execute(zjp3 delete {2})+reload(zjp3 list all)",
            "--preview",
            "zjp3 preview {}",
            "--preview-window",
            "right,60%",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("zjp3: failed to spawn fzf (is it on PATH?)")?;

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
