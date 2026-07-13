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
    pub snapshot_keep: usize,
    pub auto_snapshot_pinned: bool,
    pub attached_indicator: bool,
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
            // Snapshots per session (`noren snapshot`); oldest pruned beyond this.
            snapshot_keep: 5,
            // Auto-snapshot pinned sessions on pin and on connect.
            auto_snapshot_pinned: false,
            // ▣ vs ▢ for live sessions with/without an attached terminal.
            // Costs ~150-250ms picker startup (one parallel list-clients round
            // trip); false restores the ~20ms instant list.
            attached_indicator: true,
            default_session: DefaultSession::default(),
            session: vec![],
            wildcard: vec![],
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
    pub fn load() -> Config {
        let p = config_path();
        let Ok(raw) = std::fs::read_to_string(&p) else {
            return Config::default();
        };
        match toml::from_str(&raw) {
            Ok(cfg) => cfg,
            Err(_) => {
                eprintln!(
                    "noren: warning: failed to parse {}, using defaults",
                    p.display()
                );
                Config::default()
            }
        }
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
