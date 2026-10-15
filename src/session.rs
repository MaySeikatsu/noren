//! Pin toggle and rename for the current (or named) session.
//! Semantics match the `sn`/`zpin`/`zunpin` bash siblings — shared pin file.

use std::process::Command;

use crate::config::Config;
use crate::state;
use crate::util::sanitize_core;

fn current_session() -> String {
    std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default()
}

/// Opt-in: pinning a session takes a point-in-time backup of it. Best
/// effort — pins on names without a session are fine and stay silent.
fn maybe_snapshot(cfg: &Config, name: &str, new_state: &str) {
    if cfg.auto_snapshot_pinned && new_state == "pinned" {
        let _ = crate::snapshot::take_snapshot(cfg, name);
    }
}

/// `noren pin [name]` — toggle; defaults to the current session.
pub fn pin_cmd(cfg: &Config, name: Option<&str>) {
    let name = name
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(current_session);
    if name.is_empty() {
        eprintln!("noren pin: no name given and not inside a zellij session");
        std::process::exit(1);
    }
    let new_state = state::pin_toggle(&name);
    maybe_snapshot(cfg, &name, new_state);
    println!("{name}: {new_state}");
}

/// `noren pin-row <tsv-line>` — picker-internal pin toggle on a whole row:
/// sessions pin by name (shared file), dirs pin by path (noren-owned file),
/// anything else (config rows, the separator) is a no-op.
pub fn pin_row(cfg: &Config, line: &str) {
    let mut parts = line.split('\t');
    let source = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    match source {
        "zellij" if !name.is_empty() => {
            let s = state::pin_toggle(name);
            maybe_snapshot(cfg, name, s);
            println!("{name}: {s}");
        }
        "zoxide" if !path.is_empty() => {
            let s = state::dir_pin_toggle(path);
            println!("{path}: {s}");
        }
        _ => {}
    }
}

/// `noren rename <new>` — rename the current session.
pub fn rename_current(new: &str) {
    let old = current_session();
    if old.is_empty() {
        eprintln!("noren rename: not inside a zellij session");
        std::process::exit(1);
    }
    match rename_session(&old, new) {
        Ok(target) => println!("renamed: {old} -> {target}"),
        Err(e) => {
            eprintln!("noren rename: {e}");
            std::process::exit(1);
        }
    }
}

/// Rename a RUNNING session (zellij can't rename exited ones) and carry
/// noren's state along: pin, `last` history, snapshots, hidden flag.
/// Returns the sanitized new name.
pub fn rename_session(old: &str, new: &str) -> anyhow::Result<String> {
    // Sanitize to match zellij-autostart's rules; keep the raw name if the
    // sanitized form is empty.
    let sanitized = sanitize_core(new.trim());
    let target = if sanitized.is_empty() {
        new.trim().to_string()
    } else {
        sanitized
    };
    if target.is_empty() || target == old {
        anyhow::bail!("the new name must differ from \"{old}\"");
    }
    let sessions = crate::sources::zellij::raw_sessions();
    if sessions.iter().any(|(n, _)| *n == target) {
        anyhow::bail!("a session named \"{target}\" already exists");
    }
    if !sessions.iter().any(|(n, s)| n == old && *s == "live") {
        anyhow::bail!("only running sessions can be renamed (\"{old}\" isn't)");
    }

    let mut cmd = Command::new("zellij");
    cmd.env("ZELLIJ_SESSION_NAME", old)
        .args(["action", "rename-session", &target]);
    match crate::sources::zellij::run_with_timeout(cmd, 2000) {
        Some(o) if o.status.success() => {}
        _ => anyhow::bail!("zellij action rename-session failed"),
    }

    if state::is_pinned(old) {
        state::pin_remove(old);
        state::pin_add(&target);
    }
    state::rename_in_history(old, &target);
    state::rename_snapshots(old, &target);
    state::rename_hidden(old, &target);
    if current_session() == old {
        state::write_current_session(&target);
    }
    Ok(target)
}
