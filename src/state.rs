//! On-disk state, SHARED with sibling shell scripts — formats are contracts:
//!   ~/.local/state/zjp/last, ~/.local/state/zjp/previous  (zjp3-owned)
//!   ~/.local/state/zellij/pinned          (shared with zpin/zunpin/sn/reaper)
//!   ~/.local/state/zellij/current-session (pin-indicator fallback)

use std::fs;
use std::path::PathBuf;

use crate::util::home_dir;

fn zjp_state_dir() -> PathBuf {
    home_dir().join(".local/state/zjp")
}

fn last_file() -> PathBuf {
    zjp_state_dir().join("last")
}

fn previous_file() -> PathBuf {
    zjp_state_dir().join("previous")
}

/// Rotate: the previous "current" becomes "previous", `name` becomes "last".
pub fn record_last(name: &str) {
    let dir = zjp_state_dir();
    let _ = fs::create_dir_all(&dir);
    let curr = fs::read_to_string(last_file())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !curr.is_empty() && curr != name {
        let _ = fs::write(previous_file(), &curr);
    }
    let _ = fs::write(last_file(), name);
}

/// The session to jump back to with `zjp3 last`.
pub fn read_last() -> String {
    fs::read_to_string(previous_file())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// ---- pin files ---------------------------------------------------------------
// Session pins are SHARED with zpin/zunpin/sn/reaper (one name per line).
// Dir pins are zjp3-owned (one path per line) — folders never enter the
// shared file, so the reaper and friends only ever see session names.

fn pin_file() -> PathBuf {
    home_dir().join(".local/state/zellij/pinned")
}

fn dir_pin_file() -> PathBuf {
    zjp_state_dir().join("pinned-dirs")
}

fn read_lines(f: &PathBuf) -> Vec<String> {
    fs::read_to_string(f)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn write_lines(f: &PathBuf, entries: &[String]) {
    if let Some(dir) = f.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // Trailing newline matters: zpin appends with `echo >>`, which would glue
    // onto an unterminated last line.
    let mut body = entries.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    let _ = fs::write(f, body);
}

fn toggle_line(f: &PathBuf, entry: &str) -> &'static str {
    let entries = read_lines(f);
    if entries.iter().any(|p| p == entry) {
        let filtered: Vec<String> = entries.into_iter().filter(|p| p != entry).collect();
        write_lines(f, &filtered);
        "unpinned"
    } else {
        let mut entries = entries;
        entries.push(entry.to_string());
        write_lines(f, &entries);
        "pinned"
    }
}

pub fn read_pins() -> Vec<String> {
    read_lines(&pin_file())
}

pub fn is_pinned(name: &str) -> bool {
    read_pins().iter().any(|p| p == name)
}

/// Toggle session pin state; returns the new state.
pub fn pin_toggle(name: &str) -> &'static str {
    toggle_line(&pin_file(), name)
}

pub fn pin_add(name: &str) {
    let mut pins = read_pins();
    if !pins.iter().any(|p| p == name) {
        pins.push(name.to_string());
        write_lines(&pin_file(), &pins);
    }
}

pub fn pin_remove(name: &str) {
    let pins = read_pins();
    if pins.iter().any(|p| p == name) {
        let filtered: Vec<String> = pins.into_iter().filter(|p| p != name).collect();
        write_lines(&pin_file(), &filtered);
    }
}

pub fn read_dir_pins() -> Vec<String> {
    read_lines(&dir_pin_file())
}

/// Toggle a folder pin (by path); returns the new state.
pub fn dir_pin_toggle(path: &str) -> &'static str {
    toggle_line(&dir_pin_file(), path)
}

/// Keep the pin-indicator fallback file in sync (see zellij-pin-indicator).
/// Written without a trailing newline, matching `printf %s`.
pub fn write_current_session(name: &str) {
    let f = home_dir().join(".local/state/zellij/current-session");
    if let Some(dir) = f.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(f, name);
}
