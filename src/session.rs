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

/// `zjp3 pin [name]` — toggle; defaults to the current session.
pub fn pin_cmd(cfg: &Config, name: Option<&str>) {
    let name = name
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(current_session);
    if name.is_empty() {
        eprintln!("zjp3 pin: no name given and not inside a zellij session");
        std::process::exit(1);
    }
    let new_state = state::pin_toggle(&name);
    maybe_snapshot(cfg, &name, new_state);
    println!("{name}: {new_state}");
}

/// `zjp3 pin-row <tsv-line>` — picker-internal pin toggle on a whole row:
/// sessions pin by name (shared file), dirs pin by path (zjp3-owned file),
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

/// `zjp3 rename <new>` — rename the current session, carry the pin over.
pub fn rename_current(new: &str) {
    let old = current_session();
    if old.is_empty() {
        eprintln!("zjp3 rename: not inside a zellij session");
        std::process::exit(1);
    }
    if new.is_empty() || new == old {
        eprintln!("zjp3 rename: new name must differ from current");
        std::process::exit(1);
    }
    // Sanitize to match zellij-autostart's rules; keep the raw name if the
    // sanitized form is empty.
    let sanitized = sanitize_core(new);
    let target = if sanitized.is_empty() {
        new.to_string()
    } else {
        sanitized
    };

    let status = Command::new("zellij")
        .args(["action", "rename-session", &target])
        .status();
    match status {
        Ok(s) if s.success() => {}
        _ => {
            eprintln!("zjp3 rename: zellij action rename-session failed");
            std::process::exit(1);
        }
    }

    if state::is_pinned(&old) {
        state::pin_remove(&old);
        state::pin_add(&target);
    }
    state::write_current_session(&target);
    println!("renamed: {old} -> {target}");
}
