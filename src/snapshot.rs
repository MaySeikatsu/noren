//! Session snapshots: point-in-time copies of the serialized layout, stored
//! under ~/.local/state/noren/snapshots/<session>/<YYYYmmdd-HHMMSS>.kdl.
//!
//! Live sessions are dumped from the server (`zellij action dump-layout`,
//! targeted via ZELLIJ_SESSION_NAME like window.rs); exited sessions fall
//! back to the resurrection cache. `restore` recreates the session from a
//! stored snapshot — destructive for the current incarnation, so it confirms
//! on a tty and auto-snapshots the pre-restore state first.

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;
use crate::connect::{inside_zellij, switch_in_place};
use crate::sources::zellij::{cached_layout_file, zellij_sessions};
use crate::state::record_last;

fn session_dir(name: &str) -> PathBuf {
    crate::state::state_dir().join("snapshots").join(name)
}

/// UTC snapshot filename stamp (Hinnant's civil_from_days; no chrono dep).
fn fmt_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    fmt_utc(secs)
}

/// Snapshot files for a session, newest first. Ordered by mtime (filename
/// stamps only have second resolution).
pub fn list_snapshots(name: &str) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(session_dir(name)) else {
        return vec![];
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "kdl"))
        .map(|p| {
            let t = p
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (t, p)
        })
        .collect();
    files.sort();
    files.reverse();
    files.into_iter().map(|(_, p)| p).collect()
}

fn prune(name: &str, keep: usize) {
    for old in list_snapshots(name).into_iter().skip(keep.max(1)) {
        let _ = fs::remove_file(old);
    }
}

fn is_live(cfg: &Config, name: &str) -> bool {
    zellij_sessions(cfg)
        .iter()
        .any(|r| r.name == name && r.state == "live")
}

fn session_exists(cfg: &Config, name: &str) -> bool {
    zellij_sessions(cfg).iter().any(|r| r.name == name)
}

/// Capture the session's serialized layout. Returns the newest existing
/// snapshot unchanged content would duplicate (keeps auto-snapshots from
/// piling up identical copies).
pub fn take_snapshot(cfg: &Config, name: &str) -> anyhow::Result<PathBuf> {
    let mut body = String::new();
    if is_live(cfg, name)
        && let Ok(out) = Command::new("zellij")
            .env("ZELLIJ_SESSION_NAME", name)
            .args(["action", "dump-layout"])
            .output()
        && out.status.success()
    {
        body = String::from_utf8_lossy(&out.stdout).into_owned();
    }
    if body.trim().is_empty() {
        body = fs::read_to_string(cached_layout_file(name)).unwrap_or_default();
    }
    if body.trim().is_empty() {
        anyhow::bail!("no layout available for \"{name}\" (not live, no resurrection cache)");
    }

    let dir = session_dir(name);
    fs::create_dir_all(&dir)?;
    if let Some(newest) = list_snapshots(name).first()
        && fs::read_to_string(newest)
            .map(|s| s == body)
            .unwrap_or(false)
    {
        return Ok(newest.clone());
    }
    let stamp = timestamp();
    let mut file = dir.join(format!("{stamp}.kdl"));
    let mut n = 2;
    while file.exists() {
        file = dir.join(format!("{stamp}-{n}.kdl"));
        n += 1;
    }
    fs::write(&file, &body)?;
    prune(name, cfg.snapshot_keep);
    Ok(file)
}

fn current_or(name: Option<&str>) -> String {
    name.map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default())
}

/// `noren snapshot [name]` — default: current session.
pub fn snapshot_cmd(cfg: &Config, name: Option<&str>) {
    let name = current_or(name);
    if name.is_empty() {
        eprintln!("noren snapshot: no name given and not inside a zellij session");
        std::process::exit(1);
    }
    match take_snapshot(cfg, &name) {
        Ok(f) => println!("snapshot: {}", f.display()),
        Err(e) => {
            eprintln!("noren snapshot: {e}");
            std::process::exit(1);
        }
    }
}

/// `noren snapshots [name]` — newest first, 1-based (restore takes the index).
pub fn snapshots_cmd(name: Option<&str>) {
    let name = current_or(name);
    if name.is_empty() {
        eprintln!("noren snapshots: no name given and not inside a zellij session");
        std::process::exit(1);
    }
    let snaps = list_snapshots(&name);
    if snaps.is_empty() {
        println!("(no snapshots for \"{name}\")");
        return;
    }
    for (i, f) in snaps.iter().enumerate() {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        println!("{}: {stem} ({size} B)", i + 1);
    }
}

/// `noren restore <name> [index]` — kill + recreate the session from snapshot
/// `index` (1 = newest). The pre-restore state is snapshotted first, so a
/// mistaken restore can itself be restored from.
pub fn restore_cmd(cfg: &Config, name: &str, index: usize, force: bool) -> anyhow::Result<()> {
    let snaps = list_snapshots(name);
    if snaps.is_empty() {
        anyhow::bail!("no snapshots for \"{name}\"");
    }
    let Some(snap) = snaps.get(index.max(1) - 1) else {
        anyhow::bail!("\"{name}\" has only {} snapshot(s)", snaps.len());
    };
    let snap = snap.clone();
    let stem = snap
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let exists = session_exists(cfg, name);
    if exists && !force && std::io::stdin().is_terminal() {
        print!("Replace session \"{name}\" with snapshot {stem}? [y/N] ");
        std::io::stdout().flush().ok();
        let mut ans = String::new();
        std::io::stdin().read_line(&mut ans).ok();
        if !matches!(ans.trim(), "y" | "Y" | "yes" | "Yes") {
            println!("aborted");
            return Ok(());
        }
    }

    if exists {
        // Preserve the pre-restore state as its own snapshot (best effort).
        let _ = take_snapshot(cfg, name);
        if is_live(cfg, name) {
            let _ = Command::new("zellij").args(["kill-session", name]).status();
        }
        let _ = Command::new("zellij")
            .args(["delete-session", "--force", name])
            .status();
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    record_last(name);
    if inside_zellij() {
        let _ = Command::new("zellij")
            .args(["--session", name, "--new-session-with-layout"])
            .arg(&snap)
            .args(["--", "true"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        std::thread::sleep(std::time::Duration::from_millis(300));
        switch_in_place(name)?;
        println!("restored: {name} <- {stem}");
        Ok(())
    } else {
        use std::os::unix::process::CommandExt;
        let mut cmd = Command::new("zellij");
        cmd.args(["--session", name, "--new-session-with-layout"])
            .arg(&snap);
        Err(cmd.exec().into())
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_utc;

    #[test]
    fn utc_stamp_matches_known_instant() {
        // 2026-07-13 09:15:00 UTC
        assert_eq!(fmt_utc(1_783_934_100), "20260713-091500");
        // epoch
        assert_eq!(fmt_utc(0), "19700101-000000");
    }
}
