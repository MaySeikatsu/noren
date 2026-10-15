//! Kill / delete with a configurable confirmation policy, shared by the CLI
//! (`noren kill|delete`) and the picker binds.
//!
//! Every function here stays off stdout: the picker runs them inside fzf
//! `transform` handlers, whose stdout fzf parses as actions.

use std::process::{Command, Stdio};

use crate::config::{Config, ConfirmPolicy};
use crate::sources::zellij::{attached_clients, zellij_sessions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Kill,
    Delete,
}

impl Kind {
    pub fn verb(self) -> &'static str {
        match self {
            Kind::Kill => "kill",
            Kind::Delete => "delete",
        }
    }

    fn past(self) -> &'static str {
        match self {
            Kind::Kill => "killed",
            Kind::Delete => "deleted",
        }
    }

    fn policy(self, cfg: &Config) -> ConfirmPolicy {
        match self {
            Kind::Kill => cfg.confirm.kill,
            Kind::Delete => cfg.confirm.delete,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Missing,
    Exited,
    Live { attached: bool },
}

impl SessionState {
    /// Short qualifier for prompts/messages.
    pub fn describe(self) -> &'static str {
        match self {
            SessionState::Missing => "unknown",
            SessionState::Exited => "exited",
            SessionState::Live { attached: true } => "attached",
            SessionState::Live { attached: false } => "running",
        }
    }
}

/// Current state of `name`. Asks for attached clients of live sessions
/// only (`zellij action` against an exited session hangs).
pub fn session_state(cfg: &Config, name: &str) -> SessionState {
    let quiet = Config {
        attached_indicator: false,
        ..cfg.clone()
    };
    match zellij_sessions(&quiet).into_iter().find(|r| r.name == name) {
        None => SessionState::Missing,
        Some(r) if r.state == "live" => SessionState::Live {
            attached: attached_clients(name) > 0,
        },
        Some(_) => SessionState::Exited,
    }
}

pub fn needs_confirm(policy: ConfirmPolicy, state: SessionState) -> bool {
    match policy {
        ConfirmPolicy::Always => true,
        ConfirmPolicy::Never => false,
        ConfirmPolicy::Attached => state == SessionState::Live { attached: true },
        ConfirmPolicy::Running => matches!(state, SessionState::Live { .. }),
    }
}

/// Should `kind` on a session in `state` ask first?
pub fn should_confirm(cfg: &Config, kind: Kind, state: SessionState) -> bool {
    state != SessionState::Missing && needs_confirm(kind.policy(cfg), state)
}

/// The session this process runs in, if inside zellij.
pub fn current_session() -> Option<String> {
    let inside = std::env::var("ZELLIJ").is_ok_and(|v| !v.is_empty());
    std::env::var("ZELLIJ_SESSION_NAME")
        .ok()
        .filter(|n| inside && !n.is_empty())
}

fn zellij_quiet(args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("zellij")
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        anyhow::bail!("zellij {} failed {err}", args.join(" "))
    }
}

/// Perform `kind` on `name` (no confirmation — callers decide). Returns a
/// one-line result message. The current session is never killed under the
/// caller: callers route that case through `discard` (see `is_current`).
pub fn perform(kind: Kind, name: &str, state: SessionState) -> anyhow::Result<String> {
    match (kind, state) {
        (_, SessionState::Missing) => anyhow::bail!("no session named \"{name}\""),
        (Kind::Kill, SessionState::Exited) => Ok(format!("{name} is not running")),
        (Kind::Kill, SessionState::Live { .. }) => {
            zellij_quiet(&["kill-session", name])?;
            Ok(format!("{} {name}", kind.past()))
        }
        (Kind::Delete, SessionState::Exited) => {
            zellij_quiet(&["delete-session", name])?;
            Ok(format!("{} {name}", kind.past()))
        }
        (Kind::Delete, SessionState::Live { .. }) => {
            // --force kills the running session first.
            zellij_quiet(&["delete-session", "--force", name])?;
            Ok(format!("{} {name}", kind.past()))
        }
    }
}

pub fn is_current(name: &str) -> bool {
    current_session().is_some_and(|c| c == name)
}

/// Read a y/N answer from the terminal.
fn ask(question: &str) -> bool {
    use std::io::Write;
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut ans = String::new();
    std::io::stdin().read_line(&mut ans).ok();
    matches!(ans.trim(), "y" | "Y" | "yes" | "Yes")
}

/// `noren kill|delete <name> [--yes]`. Asks on a tty when the policy says
/// so; non-interactive callers (scripts) are never blocked by a prompt.
pub fn cli(cfg: &Config, kind: Kind, name: &str, yes: bool) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let state = session_state(cfg, name);
    if state == SessionState::Missing {
        anyhow::bail!("no session named \"{name}\"");
    }
    if !yes && std::io::stdin().is_terminal() && should_confirm(cfg, kind, state) {
        let q = format!("{} {} session \"{name}\"?", kind.verb(), state.describe());
        if !ask(&q) {
            println!("aborted");
            return Ok(());
        }
    }
    if is_current(name) && matches!(state, SessionState::Live { .. }) {
        // Leave the session before it dies, or the client is dropped.
        crate::connect::discard(cfg, Some(name))?;
        if kind == Kind::Delete {
            zellij_quiet(&["delete-session", "--force", name])?;
        }
        return Ok(());
    }
    println!("{}", perform(kind, name, state)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATT: SessionState = SessionState::Live { attached: true };
    const BG: SessionState = SessionState::Live { attached: false };
    const EX: SessionState = SessionState::Exited;

    #[test]
    fn policies() {
        use ConfirmPolicy::*;
        let table = [
            (Always, [true, true, true]),
            (Attached, [true, false, false]),
            (Running, [true, true, false]),
            (Never, [false, false, false]),
        ];
        for (p, want) in table {
            assert_eq!(
                [needs_confirm(p, ATT), needs_confirm(p, BG), needs_confirm(p, EX)],
                want,
                "{p:?}"
            );
        }
    }

    #[test]
    fn defaults_match_spec() {
        // kill asks only when attached; delete asks when running at all;
        // offline sessions are deleted right away.
        let cfg = Config::default();
        assert!(should_confirm(&cfg, Kind::Kill, ATT));
        assert!(!should_confirm(&cfg, Kind::Kill, BG));
        assert!(should_confirm(&cfg, Kind::Delete, ATT));
        assert!(should_confirm(&cfg, Kind::Delete, BG));
        assert!(!should_confirm(&cfg, Kind::Delete, EX));
        assert!(!should_confirm(&cfg, Kind::Delete, SessionState::Missing));
    }
}
