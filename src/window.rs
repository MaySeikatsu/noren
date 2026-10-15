//! `noren window` — list/switch/create tabs in a session (analog of
//! `sesh window`, thin wrapper over `zellij action`).

use std::process::Command;

use crate::sources::zellij::{ZellijOutput, run_with_timeout, session_state};
use crate::util::{expand_path, is_pathlike};

/// Kill-timeout for each `zellij action` call: a session dying mid-call (or
/// an exited one) would otherwise hang forever.
const ACTION_TIMEOUT_MS: u64 = 1500;

fn current_session() -> String {
    std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default()
}

/// `zellij action` targets the session named by ZELLIJ_SESSION_NAME; override
/// it when the caller asked for a different session. None on timeout.
fn zellij_action(session: &str, args: &[&str]) -> Option<ZellijOutput> {
    let mut cmd = Command::new("zellij");
    cmd.arg("action").args(args);
    if !session.is_empty() && session != current_session() {
        cmd.env("ZELLIJ_SESSION_NAME", session);
    }
    run_with_timeout(cmd, ACTION_TIMEOUT_MS)
}

/// Run an action whose output doesn't matter; complain when it fails.
fn run_action(session: &str, args: &[&str]) {
    match zellij_action(session, args) {
        Some(o) if o.status.success() => {}
        Some(o) => eprintln!(
            "noren window: zellij action {} failed ({})",
            args[0], o.status
        ),
        None => eprintln!(
            "noren window: zellij action {} timed out after {ACTION_TIMEOUT_MS} ms",
            args[0]
        ),
    }
}

fn list_tabs(session: &str) -> Vec<String> {
    zellij_action(session, &["query-tab-names"])
        .map(|o| {
            o.stdout
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn window(target: Option<&str>, session_flag: Option<&str>) {
    let session = session_flag
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(current_session);
    if session.is_empty() {
        eprintln!("noren window: not inside a zellij session and no --session given");
        std::process::exit(1);
    }
    // `zellij action` against an exited (or unknown) session never returns.
    if session != current_session() && session_state(&session) != Some("live") {
        eprintln!("noren window: session \"{session}\" is not running");
        std::process::exit(1);
    }

    let Some(target) = target.filter(|t| !t.is_empty()) else {
        println!("{}", list_tabs(&session).join("\n"));
        return;
    };

    if is_pathlike(target) {
        let p = expand_path(target);
        if !p.exists() {
            eprintln!("noren window: path does not exist: {}", p.display());
            std::process::exit(1);
        }
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let p = p.to_string_lossy().into_owned();
        run_action(&session, &["new-tab", "--cwd", &p, "--name", &name]);
        return;
    }

    // Named target: switch if it exists, else create an empty tab.
    let tabs = list_tabs(&session);
    if let Some(idx) = tabs.iter().position(|t| t == target) {
        let n = (idx + 1).to_string();
        run_action(&session, &["go-to-tab", &n]);
    } else {
        run_action(&session, &["new-tab", "--name", target]);
    }
}
