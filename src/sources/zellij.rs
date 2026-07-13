//! zellij source: `zellij list-sessions -n`, state live|exited, path from
//! the first `cwd "..."` in the serialized session layout, pin from pin file,
//! attached-client detection via `zellij action list-clients` (live only).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::sources::Row;
use crate::state::read_pins;
use crate::util::home_dir;

/// Resurrection cache: the serialized layout zellij keeps per session.
pub fn cached_layout_file(name: &str) -> PathBuf {
    home_dir()
        .join(".cache/zellij/contract_version_1/session_info")
        .join(name)
        .join("session-layout.kdl")
}

/// Path of the session's first tab, from the serialized layout. Cheap enough
/// for a few dozen sessions; empty string on any failure.
pub fn session_path(name: &str) -> String {
    let f = cached_layout_file(name);
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

/// (name, state) pairs from `zellij list-sessions -n` — no pin/attachment
/// lookups, cheap enough for the resolver/connect paths.
fn raw_sessions() -> Vec<(String, &'static str)> {
    let Ok(out) = Command::new("zellij")
        .args(["list-sessions", "-n"])
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let name = line.split(' ').next().unwrap_or("").to_string();
            let state = if line.contains("EXITED") {
                "exited"
            } else {
                "live"
            };
            (name, state)
        })
        .collect()
}

pub fn session_names(_cfg: &Config) -> Vec<String> {
    raw_sessions().into_iter().map(|(n, _)| n).collect()
}

/// Run `zellij action <args>` against a session with a hard timeout; None on
/// timeout/failure. The timeout matters: `zellij action` never returns when
/// the target session dies mid-flight (and hangs forever on exited ones).
fn action_output_with_timeout(name: &str, args: &[&str], ms: u64) -> Option<String> {
    let mut child = Command::new("zellij")
        .env("ZELLIJ_SESSION_NAME", name)
        .arg("action")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    let _ = so.read_to_string(&mut out);
                }
                return status.success().then_some(out);
            }
            Ok(None) => {
                if start.elapsed().as_millis() >= u128::from(ms) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return None,
        }
    }
}

/// Clients attached to a LIVE session right now (0 = running in the
/// background). Requires the CLIENT_ID header so garbage output (test stubs,
/// version skew) safely reads as detached.
pub fn attached_clients(name: &str) -> usize {
    let Some(out) = action_output_with_timeout(name, &["list-clients"], 400) else {
        return 0;
    };
    let mut lines = out.lines();
    if !lines.next().is_some_and(|h| h.starts_with("CLIENT_ID")) {
        return 0;
    }
    lines.filter(|l| !l.trim().is_empty()).count()
}

pub fn zellij_sessions(cfg: &Config) -> Vec<Row> {
    let sessions = raw_sessions();
    let pins = read_pins();

    // One list-clients round trip costs ~100ms — query all live sessions in
    // parallel so the picker startup pays for one, not one per session.
    let attached: Vec<bool> = std::thread::scope(|s| {
        sessions
            .iter()
            .map(|(name, state)| {
                (cfg.attached_indicator && *state == "live")
                    .then(|| s.spawn(move || attached_clients(name) > 0))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.is_some_and(|h| h.join().unwrap_or(false)))
            .collect()
    });

    sessions
        .into_iter()
        .zip(attached)
        .map(|((name, state), attached)| {
            let pinned = pins.iter().any(|p| p == &name);
            Row {
                source: "zellij",
                path: session_path(&name),
                state,
                pinned,
                icon: cfg.icon_for("zellij", state, pinned, attached),
                name,
            }
        })
        .collect()
}
