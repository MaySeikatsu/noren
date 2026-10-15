//! Resolver (name-or-path -> session record) and connect logic.
//!
//! Resolver precedence:
//!   1) explicit [[session]] with matching name or path
//!   2) live/exited zellij session with matching name
//!   3) [[wildcard]] match on an existing path
//!   4) fallback: sanitized basename, defaults from [default_session]
//!
//! Connect:
//!   - inside zellij: create detached if missing, then switch the client in
//!     place via the zellij-switch plugin, native `action switch-session` as
//!     fallback (never `zellij attach` -> nesting)
//!   - outside: exec zellij attach --create (or --new-session-with-layout)

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::config::Config;
use crate::sources::zellij::{
    CREATE_TIMEOUT_MS, POLL_INTERVAL_MS, POLL_TIMEOUT_MS, attached_clients, create_background,
    run_with_timeout, session_names, session_state, wait_until,
};
use crate::state::record_last;
use crate::util::{expand_path, home_dir, is_pathlike, name_from_path, sanitize};

#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub name: String,
    pub path: String,
    pub layout: String,
    pub startup_command: String,
    pub preview_command: String,
    pub source: &'static str, // config | zellij | wildcard | fallback
}

pub fn resolve(cfg: &Config, target: &str) -> Resolved {
    resolve_with(cfg, target, &session_names(cfg))
}

/// Testable core: live zellij session names are passed in.
pub fn resolve_with(cfg: &Config, target: &str, live_names: &[String]) -> Resolved {
    let d = &cfg.default_session;

    // (1) explicit [[session]] by name or path
    if let Some(s) = cfg.find_session(target) {
        let p = s
            .path
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| expand_path(p).to_string_lossy().into_owned())
            .unwrap_or_default();
        return Resolved {
            name: s
                .name
                .clone()
                .unwrap_or_else(|| sanitize(&name_from_path(&p, cfg.dir_length))),
            path: p,
            layout: s.layout.clone().unwrap_or_else(|| d.layout.clone()),
            startup_command: s
                .startup_command
                .clone()
                .unwrap_or_else(|| d.startup_command.clone()),
            preview_command: s
                .preview_command
                .clone()
                .unwrap_or_else(|| d.preview_command.clone()),
            source: "config",
        };
    }

    // (2) live/exited zellij session by name
    if live_names.iter().any(|n| n == target) {
        return Resolved {
            name: target.to_string(),
            path: String::new(),
            layout: d.layout.clone(),
            startup_command: String::new(),
            preview_command: d.preview_command.clone(),
            source: "zellij",
        };
    }

    // (3)/(4) path-like targets: wildcard match, else path fallback
    if is_pathlike(target) {
        let candidate = expand_path(target);
        if candidate.exists() {
            let p = candidate.to_string_lossy().into_owned();
            let name = sanitize(&name_from_path(&p, cfg.dir_length));
            if let Some(w) = cfg.match_wildcard(&p) {
                return Resolved {
                    name,
                    path: p,
                    layout: w.layout.clone().unwrap_or_else(|| d.layout.clone()),
                    startup_command: w
                        .startup_command
                        .clone()
                        .unwrap_or_else(|| d.startup_command.clone()),
                    preview_command: w
                        .preview_command
                        .clone()
                        .unwrap_or_else(|| d.preview_command.clone()),
                    source: "wildcard",
                };
            }
            return Resolved {
                name,
                path: p,
                layout: d.layout.clone(),
                startup_command: d.startup_command.clone(),
                preview_command: d.preview_command.clone(),
                source: "fallback",
            };
        }
    }

    // (4) fallback for a plain name
    Resolved {
        name: sanitize(target),
        path: String::new(),
        layout: d.layout.clone(),
        startup_command: String::new(),
        preview_command: String::new(),
        source: "fallback",
    }
}

fn layout_path(layout: &str) -> PathBuf {
    home_dir()
        .join(".config/zellij/layouts")
        .join(format!("{layout}.kdl"))
}

pub fn inside_zellij() -> bool {
    std::env::var("ZELLIJ")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Kill-timeout for switch calls (ms). A pipe to a missing plugin file — or
/// to one whose permission prompt is still pending — never returns.
const SWITCH_TIMEOUT_MS: u64 = 2000;

/// Switch the current client in place (never `attach` from inside —
/// nesting). Prefers the zellij-switch plugin; falls back to the native
/// `zellij action switch-session`, and errors (with a hint) instead of
/// silently doing nothing when neither works.
pub fn switch_in_place(name: &str) -> anyhow::Result<()> {
    let current = std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default();
    if name == current {
        return Ok(()); // already there
    }
    let mut why: Vec<String> = Vec::new();

    let plugin = home_dir().join(".config/zellij/plugins/zellij-switch.wasm");
    if plugin.exists() {
        let mut cmd = Command::new("zellij");
        cmd.args([
            "pipe",
            "--plugin",
            &format!("file:{}", plugin.display()),
            "--",
        ])
        .arg(format!("--session {name}"));
        match run_with_timeout(cmd, SWITCH_TIMEOUT_MS) {
            Some(o) if o.status.success() => return Ok(()),
            Some(o) => why.push(format!("zellij-switch pipe failed ({})", o.status)),
            None => why.push(format!(
                "zellij-switch pipe timed out after {SWITCH_TIMEOUT_MS} ms \
                 (plugin permissions not granted yet?)"
            )),
        }
    } else {
        why.push(format!("{} not found", plugin.display()));
    }

    // Native fallback. It never resurrects, so wake EXITED sessions first
    // (`attach --create-background` resurrects from the cache, attaches no
    // client).
    if session_state(name) == Some("exited") {
        create_background(name, None, None);
    }
    // `switch-session` exits 0 even when it moves nobody (zellij 0.45.1 from
    // the CLI), so confirm the move via our session's client count.
    let before = if current.is_empty() {
        0
    } else {
        attached_clients(&current)
    };
    let mut cmd = Command::new("zellij");
    cmd.args(["action", "switch-session", name]);
    match run_with_timeout(cmd, SWITCH_TIMEOUT_MS) {
        Some(o) if o.status.success() => {
            if before == 0
                || wait_until(1500, POLL_INTERVAL_MS, || {
                    attached_clients(&current) < before
                })
            {
                return Ok(());
            }
            why.push("`zellij action switch-session` exited 0 but no client moved".into());
        }
        Some(o) => why.push(format!(
            "`zellij action switch-session` failed ({})",
            o.status
        )),
        None => why.push(format!(
            "`zellij action switch-session` timed out after {SWITCH_TIMEOUT_MS} ms"
        )),
    }

    anyhow::bail!(
        "could not switch to session \"{name}\": {}\n\
         hint: install the zellij-switch plugin as ~/.config/zellij/plugins/zellij-switch.wasm \
         (https://github.com/mostafaqanbaryan/zellij-switch) and grant its permissions",
        why.join("; ")
    )
}

/// Attach to / create / switch to the resolved session.
pub fn session_connect(cfg: &Config, target: &str) -> anyhow::Result<()> {
    let r = resolve(cfg, target);
    let exists = session_names(cfg).iter().any(|n| n == &r.name);
    let inside = inside_zellij();

    // New sessions (and their startup command) run in the resolved path.
    let cwd: PathBuf = if !r.path.is_empty() && Path::new(&r.path).exists() {
        PathBuf::from(&r.path)
    } else {
        std::env::current_dir().unwrap_or_else(|_| home_dir())
    };

    if !exists {
        if inside {
            // Create a detached background session, then switch to it below.
            let lp = layout_path(&r.layout);
            let layout = (!r.layout.is_empty() && lp.exists()).then_some(lp.as_path());
            create_background(&r.name, layout, Some(&cwd));
        }
        if !r.startup_command.is_empty() {
            // Best effort; the pane opens in the resolved path.
            let mut cmd = Command::new("zellij");
            cmd.args([
                "--session",
                &r.name,
                "run",
                "-c",
                "--",
                "sh",
                "-c",
                &r.startup_command,
            ])
            .current_dir(&cwd);
            let _ = run_with_timeout(cmd, CREATE_TIMEOUT_MS);
        }
    }

    // Pinned sessions get a point-in-time backup on every connect (opt-in).
    if exists && cfg.auto_snapshot_pinned && crate::state::is_pinned(&r.name) {
        let _ = crate::snapshot::take_snapshot(cfg, &r.name);
    }

    if inside {
        switch_in_place(&r.name)?;
        // Only a switch that happened is history.
        record_last(&r.name);
        Ok(())
    } else {
        record_last(&r.name);
        let lp = layout_path(&r.layout);
        let mut cmd = Command::new("zellij");
        if !r.layout.is_empty() && !exists && lp.exists() {
            cmd.args(["--session", &r.name, "--new-session-with-layout"])
                .arg(&lp);
        } else {
            cmd.args(["attach", "--create", &r.name]);
        }
        // exec never returns on success.
        Err(cmd.current_dir(&cwd).exec().into())
    }
}

/// `noren discard [name]` — "close tab" for sessions: switch this client to
/// the previous (or any other live) session first, then soft-kill the
/// discarded one. The resurrection layout survives, exactly like `kill`.
pub fn discard(cfg: &Config, name: Option<&str>) -> anyhow::Result<()> {
    let current = std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default();
    let target = name
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| current.clone());
    if target.is_empty() {
        anyhow::bail!("noren discard: no name given and not inside a zellij session");
    }

    if inside_zellij() && target == current {
        let live: Vec<String> = crate::sources::zellij::zellij_sessions(cfg)
            .into_iter()
            .filter(|r| r.state == "live" && r.name != target)
            .map(|r| r.name)
            .collect();
        // Prefer the recorded previous session, else any other live one.
        let prev = crate::state::read_last();
        let fallback = if !prev.is_empty() && prev != target && live.contains(&prev) {
            prev
        } else {
            live.first().cloned().unwrap_or_default()
        };
        if !fallback.is_empty() {
            let before = attached_clients(&target);
            switch_in_place(&fallback)?;
            record_last(&fallback);
            crate::state::forget_previous(&target);
            // Let the client finish switching before its old session dies:
            // wait for this session's client count to drop (skip when it was
            // unknown/0 — nothing to wait for).
            if before > 0 {
                wait_until(POLL_TIMEOUT_MS, POLL_INTERVAL_MS, || {
                    attached_clients(&target) < before
                });
            }
        }
    }

    let status = Command::new("zellij")
        .args(["kill-session", &target])
        .status()?;
    if !status.success() {
        anyhow::bail!("zellij kill-session {target} failed");
    }
    println!("discarded: {target}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, SessionEntry, WildcardEntry};

    fn cfg_with_entries() -> Config {
        Config {
            session: vec![SessionEntry {
                name: Some("nixos".into()),
                path: Some("/tmp".into()),
                layout: Some("ide-git".into()),
                startup_command: Some("hx".into()),
                ..Default::default()
            }],
            wildcard: vec![WildcardEntry {
                pattern: Some("/tmp/**".into()),
                layout: Some("ide".into()),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn config_entry_wins_over_everything() {
        let cfg = cfg_with_entries();
        let live = vec!["nixos".to_string()];
        let r = resolve_with(&cfg, "nixos", &live);
        assert_eq!(r.source, "config");
        assert_eq!(r.layout, "ide-git");
        assert_eq!(r.path, "/tmp");
        // path match hits the same entry
        let r = resolve_with(&cfg, "/tmp", &live);
        assert_eq!(r.source, "config");
        assert_eq!(r.name, "nixos");
    }

    #[test]
    fn live_session_beats_wildcard_and_fallback() {
        let cfg = cfg_with_entries();
        let live = vec!["scratch".to_string()];
        let r = resolve_with(&cfg, "scratch", &live);
        assert_eq!(r.source, "zellij");
        assert_eq!(r.name, "scratch");
        assert_eq!(r.path, "");
    }

    #[test]
    fn wildcard_matches_existing_path() {
        // Build the pattern from the real temp dir — it isn't /tmp inside
        // the nix build sandbox.
        let tmp = std::env::temp_dir();
        let cfg = Config {
            wildcard: vec![WildcardEntry {
                pattern: Some(format!("{}/**", tmp.display())),
                layout: Some("ide".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let dir = tmp.join("noren-test-wc");
        std::fs::create_dir_all(&dir).unwrap();
        let r = resolve_with(&cfg, &dir.to_string_lossy(), &[]);
        assert_eq!(r.source, "wildcard");
        assert_eq!(r.layout, "ide");
        assert_eq!(r.name, "noren-test-wc");
    }

    #[test]
    fn nonexistent_path_falls_back_to_name() {
        let cfg = cfg_with_entries();
        let r = resolve_with(&cfg, "/definitely/not/a/real/path-xyz", &[]);
        assert_eq!(r.source, "fallback");
        // sanitized basename of the raw target string
        assert_eq!(
            r.name,
            "-definitely-not-a-real-path-xyz".trim_start_matches('-')
        );
        assert_eq!(r.path, "");
    }

    #[test]
    fn plain_name_fallback_sanitizes() {
        let cfg = Config::default();
        let r = resolve_with(&cfg, "my project!", &[]);
        assert_eq!(r.source, "fallback");
        assert_eq!(r.name, "my-project-");
    }
}
