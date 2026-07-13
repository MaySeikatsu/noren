//! `noren window` — list/switch/create tabs in a session (analog of
//! `sesh window`, thin wrapper over `zellij action`).

use std::process::Command;

use crate::util::{expand_path, is_pathlike};

fn current_session() -> String {
    std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default()
}

/// `zellij action` targets the session named by ZELLIJ_SESSION_NAME; override
/// it when the caller asked for a different session.
fn zellij_action(session: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new("zellij");
    cmd.arg("action").args(args);
    if !session.is_empty() && session != current_session() {
        cmd.env("ZELLIJ_SESSION_NAME", session);
    }
    cmd
}

fn list_tabs(session: &str) -> Vec<String> {
    zellij_action(session, &["query-tab-names"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
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
        let _ = zellij_action(&session, &["new-tab", "--cwd", &p, "--name", &name]).status();
        return;
    }

    // Named target: switch if it exists, else create an empty tab.
    let tabs = list_tabs(&session);
    if let Some(idx) = tabs.iter().position(|t| t == target) {
        let n = (idx + 1).to_string();
        let _ = zellij_action(&session, &["go-to-tab", &n]).status();
    } else {
        let _ = zellij_action(&session, &["new-tab", "--name", target]).status();
    }
}
