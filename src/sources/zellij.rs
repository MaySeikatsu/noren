//! zellij source: `zellij list-sessions -n`, state live|exited, path from
//! the first `cwd "..."` in the serialized session layout, pin from pin file.

use std::process::Command;

use crate::config::Config;
use crate::sources::Row;
use crate::state::read_pins;
use crate::util::home_dir;

/// Path of the session's first tab, from the serialized layout. Cheap enough
/// for a few dozen sessions; empty string on any failure.
pub fn session_path(name: &str) -> String {
    let f = home_dir()
        .join(".cache/zellij/contract_version_1/session_info")
        .join(name)
        .join("session-layout.kdl");
    let Ok(src) = std::fs::read_to_string(&f) else {
        return String::new();
    };
    for line in src.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("cwd") {
            if !rest.starts_with(|c: char| c.is_whitespace()) {
                continue;
            }
            if let Some(quoted) = rest.trim_start().strip_prefix('"')
                && let Some(end) = quoted.find('"')
            {
                return quoted[..end].to_string();
            }
        }
    }
    String::new()
}

pub fn session_names(cfg: &Config) -> Vec<String> {
    zellij_sessions(cfg).into_iter().map(|r| r.name).collect()
}

pub fn zellij_sessions(cfg: &Config) -> Vec<Row> {
    let Ok(out) = Command::new("zellij")
        .args(["list-sessions", "-n"])
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let pins = read_pins();
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let name = line.split(' ').next().unwrap_or("").to_string();
            let state = if line.contains("EXITED") {
                "exited"
            } else {
                "live"
            };
            let pinned = pins.iter().any(|p| p == &name);
            Row {
                source: "zellij",
                path: session_path(&name),
                state,
                pinned,
                icon: cfg.icon_for("zellij", state, pinned),
                name,
            }
        })
        .collect()
}
