//! zellij source: `zellij list-sessions -n`, state live|exited, path from
//! the first `cwd "..."` in the serialized session layout, pin from pin file,
//! attached-client detection via `zellij action list-clients` (live only).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

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

/// Output of a zellij call that finished within its timeout.
pub(crate) struct ZellijOutput {
    pub status: ExitStatus,
    pub stdout: String,
}

/// Run a zellij `Command` with a hard kill-timeout; None on spawn failure or
/// timeout. stdout is drained by a reader thread while waiting, so a large
/// `dump-layout` can't fill the pipe buffer and deadlock. stdin and stderr
/// are nulled.
pub(crate) fn run_with_timeout(mut cmd: Command, ms: u64) -> Option<ZellijOutput> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut so = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = so.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let buf = reader.join().unwrap_or_default();
                return Some(ZellijOutput {
                    status,
                    stdout: String::from_utf8_lossy(&buf).into_owned(),
                });
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                // Never join the reader here: a grandchild may still hold the
                // pipe open. The thread dies with the process.
                return None;
            }
        }
    }
}

/// Poll `cond` every `interval_ms` until it holds or `timeout_ms` elapses.
/// Returns whether it held — callers usually proceed either way.
pub(crate) fn wait_until(
    timeout_ms: u64,
    interval_ms: u64,
    mut cond: impl FnMut() -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

/// (name, state) pairs from `zellij list-sessions -n` — no pin/attachment
/// lookups, cheap enough for the resolver/connect paths.
pub(crate) fn raw_sessions() -> Vec<(String, &'static str)> {
    let mut cmd = Command::new("zellij");
    cmd.args(["list-sessions", "-n"]);
    let Some(out) = run_with_timeout(cmd, 3000) else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    out.stdout
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

/// "live" | "exited" for a session zellij lists, None otherwise.
pub(crate) fn session_state(name: &str) -> Option<&'static str> {
    raw_sessions()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, s)| s)
}

pub fn session_names(_cfg: &Config) -> Vec<String> {
    raw_sessions().into_iter().map(|(n, _)| n).collect()
}

/// Readiness polling (replaces fixed sleeps) and the creation kill-timeout.
pub(crate) const POLL_TIMEOUT_MS: u64 = 2500;
pub(crate) const POLL_INTERVAL_MS: u64 = 25;
pub(crate) const CREATE_TIMEOUT_MS: u64 = 5000;

/// Wait until zellij lists `name` as live — or no longer lists it at all
/// (`present` = false). Proceeds after the timeout either way.
pub(crate) fn wait_for_session(name: &str, present: bool) -> bool {
    wait_until(POLL_TIMEOUT_MS, POLL_INTERVAL_MS, || {
        let state = session_state(name);
        if present {
            state == Some("live")
        } else {
            state.is_none()
        }
    })
}

/// Create (or resurrect, if EXITED) a session in the background — no client
/// attaches, so it is safe from inside zellij — then wait until it is live.
/// `layout` is a layout file; resurrection ignores it and uses the cache.
/// (`--new-session-with-layout L -- true`, used before, is rejected by
/// zellij 0.45: "unrecognized subcommand 'true'".)
pub(crate) fn create_background(name: &str, layout: Option<&Path>, cwd: Option<&Path>) -> bool {
    let mut cmd = Command::new("zellij");
    cmd.args(["attach", "--create-background", name]);
    if let Some(l) = layout {
        cmd.args(["options", "--default-layout"]).arg(l);
    }
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let _ = run_with_timeout(cmd, CREATE_TIMEOUT_MS);
    // Measured on 0.45.1: listed ~35 ms after the command returns (~90 ms).
    wait_for_session(name, true)
}

/// Run `zellij action <args>` against a session with a hard timeout; None on
/// timeout/failure. The timeout matters: `zellij action` never returns when
/// the target session dies mid-flight (and hangs forever on exited ones).
pub(crate) fn action_output_with_timeout(name: &str, args: &[&str], ms: u64) -> Option<String> {
    let mut cmd = Command::new("zellij");
    cmd.env("ZELLIJ_SESSION_NAME", name)
        .arg("action")
        .args(args);
    run_with_timeout(cmd, ms)
        .filter(|o| o.status.success())
        .map(|o| o.stdout)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_with_timeout_kills_hung_commands() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exec sleep 30"]);
        let start = Instant::now();
        assert!(run_with_timeout(cmd, 200).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn run_with_timeout_survives_a_grandchild_holding_stdout() {
        // `sleep` (not exec'd) keeps the pipe open after sh is killed.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30; true"]);
        let start = Instant::now();
        assert!(run_with_timeout(cmd, 200).is_none());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn run_with_timeout_drains_large_output() {
        // Far beyond the 64 KiB pipe buffer: must not deadlock.
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "i=0; while [ $i -lt 20000 ]; do echo 0123456789abcdef; i=$((i+1)); done",
        ]);
        let out = run_with_timeout(cmd, 10_000).expect("finished in time");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 20_000 * 17);
    }

    #[test]
    fn wait_until_reports_success_and_timeout() {
        let mut n = 0;
        assert!(wait_until(1000, 1, || {
            n += 1;
            n >= 3
        }));
        assert_eq!(n, 3);
        let start = Instant::now();
        assert!(!wait_until(50, 5, || false));
        assert!(start.elapsed() >= Duration::from_millis(50));
    }
}
