//! Config loader for ~/.config/noren/config.toml (falling back to the
//! pre-rename ~/.config/zjp/config.toml, still shared with zjp2 during the
//! bake-off). Schema parity with zjp2 (see config.toml.example); missing
//! keys fall back to defaults, a malformed file warns and uses defaults.

use serde::Deserialize;
use std::path::PathBuf;

use crate::util::{expand_path, home_dir};

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub sort_order: Vec<String>,
    pub session_order: Vec<String>,
    pub separator: bool,
    pub pinned_dirs_with_sessions: bool,
    pub preview_mode: String,
    pub blacklist: Vec<String>,
    pub dir_length: usize,
    pub icons: bool,
    pub show_paths: bool,
    pub snapshot_keep: usize,
    pub auto_snapshot_pinned: bool,
    pub attached_indicator: bool,
    pub keys: Keys,
    pub confirm: Confirm,
    pub default_session: DefaultSession,
    pub session: Vec<SessionEntry>,
    pub wildcard: Vec<WildcardEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // The first emitted row sits next to the fzf prompt/text field,
            // so sessions come first (they have priority), folders last.
            sort_order: vec!["zellij".into(), "config".into(), "zoxide".into()],
            // Within the zellij group, again closest-to-prompt first.
            session_order: vec!["pinned".into(), "live".into(), "exited".into()],
            // Light hairline between session rows and folder rows.
            separator: true,
            // Pinned dirs stay in the folder area by default; opt in to move
            // them next to the pinned sessions.
            pinned_dirs_with_sessions: false,
            // "visual" pane-box diagram, or "text" for the tree render.
            preview_mode: "visual".into(),
            blacklist: vec![],
            dir_length: 1,
            icons: true,
            // Dimmed path after each name in the picker (disambiguates
            // random session names and same-basename folders).
            show_paths: true,
            // Snapshots per session (`noren snapshot`); oldest pruned beyond this.
            snapshot_keep: 5,
            // Auto-snapshot pinned sessions on pin and on connect.
            auto_snapshot_pinned: false,
            // ▣ vs ▢ for live sessions with/without an attached terminal.
            // Costs ~150-250ms picker startup (one parallel list-clients round
            // trip); false restores the ~20ms instant list.
            attached_indicator: true,
            keys: Keys::default(),
            confirm: Confirm::default(),
            default_session: DefaultSession::default(),
            session: vec![],
            wildcard: vec![],
        }
    }
}

/// Picker keybinds (fzf key names, e.g. "ctrl-t", "alt-k"); "" disables a
/// bind. The picker header is generated from these, so it always shows the
/// active set.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub sessions: String,
    pub dirs: String,
    pub config: String,
    pub all: String,
    pub reload: String,
    pub pin: String,
    pub snapshot: String,
    pub kill: String,
    pub delete: String,
    pub preview: String,
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            sessions: "ctrl-t".into(),
            dirs: "ctrl-z".into(),
            config: "ctrl-f".into(),
            all: "ctrl-a".into(),
            reload: "ctrl-r".into(),
            pin: "ctrl-p".into(),
            snapshot: "ctrl-s".into(),
            kill: "ctrl-k".into(),
            delete: "ctrl-d".into(),
            preview: "ctrl-/".into(),
        }
    }
}

impl Keys {
    /// (action, key) pairs in header order, disabled binds skipped.
    pub fn active(&self) -> Vec<(&'static str, &str)> {
        [
            ("sessions", self.sessions.as_str()),
            ("dirs", self.dirs.as_str()),
            ("config", self.config.as_str()),
            ("all", self.all.as_str()),
            ("reload", self.reload.as_str()),
            ("pin", self.pin.as_str()),
            ("snapshot", self.snapshot.as_str()),
            ("kill", self.kill.as_str()),
            ("delete", self.delete.as_str()),
            ("preview", self.preview.as_str()),
        ]
        .into_iter()
        .filter(|(_, k)| !k.trim().is_empty())
        .collect()
    }
}

/// When kill/delete ask before acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfirmPolicy {
    /// Every time.
    Always,
    /// Only when a terminal shows the session right now (▣).
    Attached,
    /// Whenever the session is running (attached or in the background).
    Running,
    /// Never.
    Never,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Confirm {
    pub kill: ConfirmPolicy,
    pub delete: ConfirmPolicy,
}

impl Default for Confirm {
    fn default() -> Self {
        // Killing a background session is cheap to undo (it resurrects);
        // deleting a running one throws away live work.
        Confirm {
            kill: ConfirmPolicy::Attached,
            delete: ConfirmPolicy::Running,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DefaultSession {
    pub startup_command: String,
    pub preview_command: String,
    pub layout: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SessionEntry {
    pub name: Option<String>,
    pub path: Option<String>,
    pub startup_command: Option<String>,
    pub preview_command: Option<String>,
    pub layout: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WildcardEntry {
    pub pattern: Option<String>,
    pub startup_command: Option<String>,
    pub preview_command: Option<String>,
    pub layout: Option<String>,
}

fn config_path() -> PathBuf {
    let preferred = home_dir().join(".config/noren/config.toml");
    if preferred.exists() {
        return preferred;
    }
    home_dir().join(".config/zjp/config.toml")
}

impl Config {
    /// Load the config; problems go to stderr (suppressed under
    /// NOREN_QUIET, which the picker sets for its own re-execs — it shows
    /// the warnings in its header instead).
    pub fn load() -> Config {
        let (cfg, warnings) = Config::load_with_warnings();
        if std::env::var_os("NOREN_QUIET").is_none() {
            for w in &warnings {
                eprintln!("noren: {w}");
            }
        }
        cfg
    }

    /// Config plus human-readable problems (parse errors with position,
    /// unknown keys, invalid values). A malformed file yields defaults.
    pub fn load_with_warnings() -> (Config, Vec<String>) {
        let p = config_path();
        let Ok(raw) = std::fs::read_to_string(&p) else {
            return (Config::default(), vec![]);
        };
        parse_with_warnings(&raw, &p.display().to_string())
    }

    /// Find a [[session]] entry matching either name or (expanded) path.
    pub fn find_session(&self, target: &str) -> Option<&SessionEntry> {
        let target_expanded = expand_path(target);
        self.session.iter().find(|s| {
            let name_match = s.name.as_deref() == Some(target);
            let path_match = s
                .path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| expand_path(p) == target_expanded)
                .unwrap_or(false);
            name_match || path_match
        })
    }

    /// First [[wildcard]] entry whose pattern matches the (expanded) path.
    pub fn match_wildcard(&self, target_path: &str) -> Option<&WildcardEntry> {
        let p = expand_path(target_path).to_string_lossy().into_owned();
        self.wildcard.iter().find(|w| {
            let Some(pat) = w.pattern.as_deref().filter(|p| !p.is_empty()) else {
                return false;
            };
            let pat = expand_path(pat).to_string_lossy().into_owned();
            wildcard_match(&pat, &p)
        })
    }

    /// Icon column for a row. Empty strings when icons are disabled.
    /// `attached` only applies to live zellij sessions: ▣ = shown in a
    /// terminal right now, ▢ = running in the background.
    pub fn icon_for(&self, source: &str, state: &str, pinned: bool, attached: bool) -> String {
        if !self.icons {
            return String::new();
        }
        let base = match source {
            "zellij" => match (state, attached) {
                ("live", true) => "▣ ",
                ("live", false) => "▢ ",
                _ => "⊗ ",
            },
            "zoxide" => "📁 ",
            "config" => "⚙ ",
            _ => "",
        };
        let pin = if pinned { "📌 " } else { "" };
        format!("{pin}{base}")
    }
}

/// Known keys per table, for unknown-key warnings (serde silently ignores
/// them, so a typo like `sort_oder` would otherwise do nothing).
const TOP_KEYS: &[&str] = &[
    "sort_order",
    "session_order",
    "separator",
    "pinned_dirs_with_sessions",
    "preview_mode",
    "blacklist",
    "dir_length",
    "icons",
    "show_paths",
    "snapshot_keep",
    "auto_snapshot_pinned",
    "attached_indicator",
    "keys",
    "confirm",
    "default_session",
    "session",
    "wildcard",
];
const KEYS_KEYS: &[&str] = &[
    "sessions", "dirs", "config", "all", "reload", "pin", "snapshot", "kill", "delete", "preview",
];
const CONFIRM_KEYS: &[&str] = &["kill", "delete"];
const DEFAULT_SESSION_KEYS: &[&str] = &["startup_command", "preview_command", "layout"];
const SESSION_KEYS: &[&str] = &["name", "path", "startup_command", "preview_command", "layout"];
const WILDCARD_KEYS: &[&str] = &["pattern", "startup_command", "preview_command", "layout"];

fn unknown_keys(table: &toml::Table, known: &[&str], prefix: &str, out: &mut Vec<String>) {
    for k in table.keys() {
        if !known.contains(&k.as_str()) {
            out.push(format!("unknown config key `{prefix}{k}` (ignored)"));
        }
    }
}

/// Parse `raw` (from `origin`, for messages) into a Config plus warnings.
pub fn parse_with_warnings(raw: &str, origin: &str) -> (Config, Vec<String>) {
    let cfg: Config = match toml::from_str(raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            // toml's message is multi-line (position + snippet); the first
            // line carries "line X, column Y" and the reason.
            let msg = e.to_string();
            let msg = msg
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            return (
                Config::default(),
                vec![format!("config error in {origin}: {msg} — using defaults")],
            );
        }
    };
    let mut w = Vec::new();
    if let Ok(table) = raw.parse::<toml::Table>() {
        unknown_keys(&table, TOP_KEYS, "", &mut w);
        let sub = |key: &str| table.get(key).and_then(|v| v.as_table());
        if let Some(t) = sub("keys") {
            unknown_keys(t, KEYS_KEYS, "keys.", &mut w);
        }
        if let Some(t) = sub("confirm") {
            unknown_keys(t, CONFIRM_KEYS, "confirm.", &mut w);
        }
        if let Some(t) = sub("default_session") {
            unknown_keys(t, DEFAULT_SESSION_KEYS, "default_session.", &mut w);
        }
        for (key, known) in [("session", SESSION_KEYS), ("wildcard", WILDCARD_KEYS)] {
            let arr = table.get(key).and_then(|v| v.as_array());
            for (i, t) in arr.into_iter().flatten().filter_map(|v| v.as_table()).enumerate() {
                unknown_keys(t, known, &format!("{key}[{i}]."), &mut w);
            }
        }
    }
    if !matches!(cfg.preview_mode.as_str(), "visual" | "text") {
        w.push(format!(
            "preview_mode = \"{}\" is not \"visual\" or \"text\" (using visual)",
            cfg.preview_mode
        ));
    }
    for o in &cfg.sort_order {
        if !matches!(o.as_str(), "zellij" | "config" | "zoxide") {
            w.push(format!("sort_order: unknown group \"{o}\" (zellij|config|zoxide)"));
        }
    }
    for o in &cfg.session_order {
        if !matches!(o.as_str(), "pinned" | "live" | "exited") {
            w.push(format!("session_order: unknown group \"{o}\" (pinned|live|exited)"));
        }
    }
    let active = cfg.keys.active();
    for (i, (action, key)) in active.iter().enumerate() {
        if let Some((other, _)) = active[..i].iter().find(|(_, k)| k == key) {
            w.push(format!(
                "keys: \"{key}\" is bound to both `{other}` and `{action}` (`{action}` wins)"
            ));
        }
        if matches!(*key, "enter" | "esc" | "change" | "start") {
            w.push(format!("keys.{action} = \"{key}\" is reserved by the picker"));
        }
    }
    (cfg, w)
}

/// Glob matcher with zjp2's semantics (config.nu match-wildcard):
///   `*`  -> [^/]*    `**` -> .*    `?` -> [^/]
/// anchored to the full string.
pub fn wildcard_match(pattern: &str, s: &str) -> bool {
    #[derive(Clone, Copy)]
    enum Pt {
        Lit(char),
        AnyChar, // ?
        AnySeg,  // *
        AnyDeep, // **
    }
    let mut pat = Vec::new();
    let cs: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '*' if i + 1 < cs.len() && cs[i + 1] == '*' => {
                pat.push(Pt::AnyDeep);
                i += 2;
            }
            '*' => {
                pat.push(Pt::AnySeg);
                i += 1;
            }
            '?' => {
                pat.push(Pt::AnyChar);
                i += 1;
            }
            c => {
                pat.push(Pt::Lit(c));
                i += 1;
            }
        }
    }
    let text: Vec<char> = s.chars().collect();

    fn match_at(pat: &[Pt], s: &[char]) -> bool {
        match pat.first() {
            None => s.is_empty(),
            Some(Pt::Lit(c)) => s.first() == Some(c) && match_at(&pat[1..], &s[1..]),
            Some(Pt::AnyChar) => {
                s.first().is_some_and(|c| *c != '/') && match_at(&pat[1..], &s[1..])
            }
            Some(Pt::AnySeg) => {
                let mut k = 0;
                loop {
                    if match_at(&pat[1..], &s[k..]) {
                        return true;
                    }
                    if k < s.len() && s[k] != '/' {
                        k += 1;
                    } else {
                        return false;
                    }
                }
            }
            Some(Pt::AnyDeep) => {
                let mut k = 0;
                loop {
                    if match_at(&pat[1..], &s[k..]) {
                        return true;
                    }
                    if k < s.len() {
                        k += 1;
                    } else {
                        return false;
                    }
                }
            }
        }
    }
    match_at(&pat, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_single_segment() {
        assert!(wildcard_match("/p/*", "/p/foo"));
        assert!(!wildcard_match("/p/*", "/p/foo/bar"));
        assert!(wildcard_match("/p/*/src", "/p/foo/src"));
    }

    #[test]
    fn wildcard_recursive() {
        assert!(wildcard_match("/p/**", "/p/foo"));
        assert!(wildcard_match("/p/**", "/p/foo/bar/baz"));
    }

    #[test]
    fn wildcard_question_mark() {
        assert!(wildcard_match("/p/fo?", "/p/foo"));
        assert!(!wildcard_match("/p/fo?", "/p/fo/"));
        assert!(!wildcard_match("/p/f?o", "/p/f/o"));
    }

    #[test]
    fn wildcard_is_anchored() {
        assert!(!wildcard_match("/p/foo", "/p/foobar"));
        assert!(!wildcard_match("/p/foo", "x/p/foo"));
        assert!(wildcard_match("/p/foo", "/p/foo"));
    }

    #[test]
    fn wildcard_literal_dots_are_not_regex() {
        assert!(!wildcard_match("/p/a.c", "/p/abc"));
        assert!(wildcard_match("/p/a.c", "/p/a.c"));
    }

    #[test]
    fn find_session_by_name_and_path() {
        let cfg = Config {
            session: vec![SessionEntry {
                name: Some("nixos".into()),
                path: Some("/etc/nixos".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(cfg.find_session("nixos").is_some());
        assert!(cfg.find_session("/etc/nixos").is_some());
        assert!(cfg.find_session("/etc/other").is_none());
        assert!(cfg.find_session("other").is_none());
    }

    #[test]
    fn icons_toggle() {
        let mut cfg = Config::default();
        assert_eq!(cfg.icon_for("zellij", "live", true, false), "📌 ▢ ");
        assert_eq!(cfg.icon_for("zellij", "live", false, true), "▣ ");
        assert_eq!(cfg.icon_for("zellij", "exited", false, false), "⊗ ");
        assert_eq!(cfg.icon_for("zoxide", "dir", false, false), "📁 ");
        assert_eq!(cfg.icon_for("config", "config", false, false), "⚙ ");
        cfg.icons = false;
        assert_eq!(cfg.icon_for("zellij", "live", true, true), "");
    }
}
