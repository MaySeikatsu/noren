//! Session sources (zellij / zoxide / config), merged into one row list.
//! Row schema and TSV format are contracts with the fzf picker and preview.

pub mod config_src;
pub mod zellij;
pub mod zoxide;

use crate::config::{Config, wildcard_match};
use crate::util::{expand_path, name_from_path, sanitize};

#[derive(Debug, Clone)]
pub struct Row {
    pub source: &'static str, // zellij | zoxide | config
    pub name: String,
    pub path: String,
    pub state: &'static str, // live | exited | dir | config
    pub pinned: bool,
    pub icon: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Selection {
    pub zellij: bool,
    pub zoxide: bool,
    pub config: bool,
    /// true INVERTS the blacklist: show only blacklisted entries.
    pub blacklisted: bool,
}

impl Selection {
    pub fn all() -> Self {
        Selection {
            zellij: true,
            zoxide: true,
            config: true,
            blacklisted: false,
        }
    }
}

/// Keyword-based source selector, parity with zjp2 `parse-source`.
pub fn parse_source(word: &str) -> Selection {
    match word.to_lowercase().as_str() {
        "zellij" | "z" | "sessions" | "t" => Selection {
            zellij: true,
            zoxide: false,
            config: false,
            blacklisted: false,
        },
        "zoxide" | "dirs" => Selection {
            zellij: false,
            zoxide: true,
            config: false,
            blacklisted: false,
        },
        "config" | "c" => Selection {
            zellij: false,
            zoxide: false,
            config: true,
            blacklisted: false,
        },
        "blacklist" | "blacklisted" | "b" => Selection {
            zellij: true,
            zoxide: true,
            config: true,
            blacklisted: true,
        },
        _ => Selection::all(),
    }
}

/// Dim hairline row separating session rows from folder rows.
pub const SEPARATOR_DISPLAY: &str = "\x1b[2m╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌\x1b[0m";

fn separator_row() -> Row {
    Row {
        source: "sep",
        name: String::new(),
        path: String::new(),
        state: "sep",
        pinned: false,
        icon: String::new(),
    }
}

/// The first emitted row renders next to the fzf prompt, so "session area"
/// rows (sessions, plus pinned dirs when opted in) come first.
fn in_session_area(r: &Row, cfg: &Config) -> bool {
    r.source == "zellij" || (r.source == "zoxide" && r.pinned && cfg.pinned_dirs_with_sessions)
}

/// Stable sort: groups by cfg.sort_order (unknown sources last), the zellij
/// group internally by cfg.session_order ("pinned" | "live" | "exited",
/// closest-to-prompt first). Pinned dirs float to the front of their group,
/// or — with pinned_dirs_with_sessions — slot in right after the pinned
/// sessions.
pub fn sort_rows(rows: &mut [Row], cfg: &Config) {
    let group_of = |source: &str| {
        cfg.sort_order
            .iter()
            .position(|o| o == source)
            .unwrap_or(999)
    };
    let session_rank = |sub: &str| {
        cfg.session_order
            .iter()
            .position(|s| s == sub)
            .unwrap_or(499)
            * 2
    };
    rows.sort_by_key(|r| match r.source {
        "zellij" => {
            let sub = if r.pinned { "pinned" } else { r.state };
            (group_of("zellij"), session_rank(sub))
        }
        "zoxide" if r.pinned => {
            if cfg.pinned_dirs_with_sessions {
                (group_of("zellij"), session_rank("pinned") + 1)
            } else {
                (group_of("zoxide"), 0)
            }
        }
        "zoxide" => (group_of("zoxide"), 1),
        other => (group_of(other), 0),
    });
}

/// Normalized identity of a row's path ("" stays "" — never an identity).
fn path_key(p: &str) -> String {
    if p.is_empty() {
        String::new()
    } else {
        expand_path(p).to_string_lossy().into_owned()
    }
}

/// Dedupe by identity, keeping the first occurrence within each kind
/// (callers sort before deduping):
///   sessions/config — by name; a session hides a config row of that name.
///   dirs — by path; hidden when a session or config row has the same path,
///   or when a session without a cached path yet (just created) has the
///   name the dir would resolve to.
/// Cross-kind hiding does not depend on position, so a pinned dir sorted
/// ahead of the sessions still yields to them: sessions beat config beat dirs.
pub fn dedupe(rows: Vec<Row>, cfg: &Config) -> Vec<Row> {
    use std::collections::HashSet;
    let session_names: HashSet<String> = rows
        .iter()
        .filter(|r| r.source == "zellij")
        .map(|r| r.name.clone())
        .collect();
    let pathless_sessions: HashSet<String> = rows
        .iter()
        .filter(|r| r.source == "zellij" && r.path.is_empty())
        .map(|r| r.name.clone())
        .collect();
    let claimed_paths: HashSet<String> = rows
        .iter()
        .filter(|r| r.source != "zoxide")
        .map(|r| path_key(&r.path))
        .filter(|p| !p.is_empty())
        .collect();

    let mut seen_names = HashSet::new();
    let mut seen_paths = HashSet::new();
    rows.into_iter()
        .filter(|r| match r.source {
            "zoxide" => {
                let key = path_key(&r.path);
                if key.is_empty() {
                    return true;
                }
                let would_be = sanitize(&name_from_path(&key, cfg.dir_length));
                !claimed_paths.contains(&key)
                    && !pathless_sessions.contains(&would_be)
                    && seen_paths.insert(key)
            }
            "zellij" => seen_names.insert(r.name.clone()),
            _ => !session_names.contains(&r.name) && seen_names.insert(r.name.clone()),
        })
        .collect()
}

/// Blacklist match. Sessions and config rows match by name; dirs only by
/// path — an entry equal to the path (`~` expanded) or matching it as a
/// glob (`*`/`**`/`?`, same as [[wildcard]]), never by basename.
fn is_blacklisted(r: &Row, patterns: &[String], names: &[String]) -> bool {
    if r.source == "zoxide" {
        let p = path_key(&r.path);
        !p.is_empty() && patterns.iter().any(|pat| wildcard_match(pat, &p))
    } else {
        names.iter().any(|b| b == &r.name)
    }
}

/// Load and merge the selected sources, sorted, deduped, blacklist-filtered,
/// with a hairline row inserted where the session area ends.
pub fn collect(cfg: &Config, sel: Selection) -> Vec<Row> {
    let mut rows = Vec::new();
    if sel.config {
        rows.extend(config_src::config_sessions(cfg));
    }
    if sel.zellij {
        rows.extend(zellij::zellij_sessions(cfg));
    }
    if sel.zoxide {
        rows.extend(zoxide::zoxide_dirs(cfg));
    }

    sort_rows(&mut rows, cfg);
    let deduped = dedupe(rows, cfg);

    // Path-like entries are expanded once; bare words stay as written (no
    // cwd join) and only match dirs as globs, e.g. "**/scratch".
    let bl = &cfg.blacklist;
    let patterns: Vec<String> = bl
        .iter()
        .map(|b| {
            if b.starts_with('/') || b.starts_with('~') {
                path_key(b)
            } else {
                b.clone()
            }
        })
        .collect();
    let mut rows: Vec<Row> = deduped
        .into_iter()
        .filter(|r| is_blacklisted(r, &patterns, bl) == sel.blacklisted)
        .collect();

    if cfg.separator {
        // Insert at every boundary between session-area and folder rows
        // (single-source views have no boundary, so no line).
        let mut i = rows.len();
        while i > 1 {
            i -= 1;
            if in_session_area(&rows[i], cfg) != in_session_area(&rows[i - 1], cfg) {
                rows.insert(i, separator_row());
            }
        }
    }
    rows
}

/// TSV columns: source, name, path, state, display ("<icon><name>").
pub fn rows_to_tsv(rows: &[Row]) -> String {
    rows.iter()
        .map(|r| {
            if r.source == "sep" {
                return format!("sep\t\t\tsep\t{SEPARATOR_DISPLAY}");
            }
            format!(
                "{}\t{}\t{}\t{}\t{}{}",
                r.source, r.name, r.path, r.state, r.icon, r.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `noren list <word>` — print rows for the selected source(s).
pub fn list_cmd(cfg: &Config, word: &str) {
    let rows = collect(cfg, parse_source(word));
    println!("{}", rows_to_tsv(&rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source: &'static str, name: &str, pinned: bool) -> Row {
        Row {
            source,
            name: name.into(),
            path: String::new(),
            state: "live",
            pinned,
            icon: String::new(),
        }
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    fn row_state(source: &'static str, name: &str, state: &'static str, pinned: bool) -> Row {
        Row {
            state,
            ..row(source, name, pinned)
        }
    }

    #[test]
    fn sort_sessions_first_pinned_closest_to_prompt() {
        let cfg = Config::default();
        let mut rows = vec![
            row_state("zoxide", "d1", "dir", false),
            row_state("config", "c1", "config", false),
            row_state("zellij", "dead", "exited", false),
            row_state("zellij", "open", "live", false),
            row_state("zellij", "pin", "live", true),
        ];
        sort_rows(&mut rows, &cfg);
        // First row sits next to the fzf text field.
        assert_eq!(names(&rows), vec!["pin", "open", "dead", "c1", "d1"]);
    }

    #[test]
    fn session_order_is_configurable() {
        let cfg = Config {
            session_order: vec!["exited".into(), "live".into(), "pinned".into()],
            ..Default::default()
        };
        let mut rows = vec![
            row_state("zellij", "pin", "live", true),
            row_state("zellij", "open", "live", false),
            row_state("zellij", "dead", "exited", false),
        ];
        sort_rows(&mut rows, &cfg);
        assert_eq!(names(&rows), vec!["dead", "open", "pin"]);
    }

    #[test]
    fn pinned_dirs_float_to_front_of_dir_group() {
        let cfg = Config::default();
        let mut rows = vec![
            row_state("zoxide", "d1", "dir", false),
            row_state("zoxide", "dp", "dir", true),
            row_state("zellij", "s1", "live", false),
        ];
        sort_rows(&mut rows, &cfg);
        assert_eq!(names(&rows), vec!["s1", "dp", "d1"]);
    }

    #[test]
    fn pinned_dirs_can_join_the_session_pins() {
        let cfg = Config {
            pinned_dirs_with_sessions: true,
            ..Default::default()
        };
        let mut rows = vec![
            row_state("zoxide", "dp", "dir", true),
            row_state("zellij", "open", "live", false),
            row_state("zellij", "pin", "live", true),
        ];
        sort_rows(&mut rows, &cfg);
        // Right after the pinned sessions, still before live ones.
        assert_eq!(names(&rows), vec!["pin", "dp", "open"]);
    }

    #[test]
    fn unknown_sources_sort_last_and_stay_stable() {
        let cfg = Config::default();
        let mut rows = vec![
            row_state("mystery", "m1", "dir", false),
            row_state("zoxide", "d1", "dir", false),
        ];
        sort_rows(&mut rows, &cfg);
        assert_eq!(names(&rows), vec!["d1", "m1"]);
    }

    fn at(source: &'static str, name: &str, path: &str) -> Row {
        Row {
            path: path.into(),
            ..row(source, name, false)
        }
    }

    fn sources_paths(rows: &[Row]) -> Vec<(&str, &str)> {
        rows.iter().map(|r| (r.source, r.path.as_str())).collect()
    }

    #[test]
    fn dedupe_keeps_same_basename_dirs_but_drops_equal_paths() {
        let cfg = Config::default();
        let rows = vec![
            at("zoxide", "scratch", "/a/scratch"),
            at("zoxide", "scratch", "/b/scratch"),
            at("zoxide", "scratch", "/a/scratch/"),
        ];
        let out = dedupe(rows, &cfg);
        assert_eq!(
            sources_paths(&out),
            vec![("zoxide", "/a/scratch"), ("zoxide", "/b/scratch")]
        );
    }

    #[test]
    fn dedupe_sessions_by_name_and_session_beats_config() {
        let cfg = Config::default();
        let rows = vec![
            at("zellij", "proj", "/x"),
            at("zellij", "proj", "/y"),
            at("config", "proj", "/z"),
            // Same path as a session, different name: config rows stay.
            at("config", "other", "/x"),
            at("config", "other", "/w"),
        ];
        let out = dedupe(rows, &cfg);
        assert_eq!(
            sources_paths(&out),
            vec![("zellij", "/x"), ("config", "/x")]
        );
    }

    #[test]
    fn dedupe_hides_dirs_claimed_by_session_or_config_path() {
        let cfg = Config::default();
        let rows = vec![
            at("zellij", "cubic-bee", "/home/u/nixos"),
            at("config", "notes", "/home/u/notes"),
            at("zoxide", "nixos", "/home/u/nixos"),
            at("zoxide", "notes", "/home/u/notes"),
            at("zoxide", "other", "/home/u/other"),
        ];
        let out = dedupe(rows, &cfg);
        assert_eq!(names(&out), vec!["cubic-bee", "notes", "other"]);
        assert_eq!(out[1].source, "config");
    }

    #[test]
    fn dedupe_hides_dir_of_just_created_pathless_session() {
        let cfg = Config::default();
        let rows = vec![
            at("zellij", "my-proj", ""),
            // Resolves to sanitize("my proj") == "my-proj".
            at("zoxide", "my proj", "/p/my proj"),
            at("zoxide", "proj", "/p/proj"),
        ];
        let out = dedupe(rows, &cfg);
        assert_eq!(names(&out), vec!["my-proj", "proj"]);
    }

    #[test]
    fn dedupe_cross_kind_is_order_independent() {
        // pinned_dirs_with_sessions can sort a dir ahead of live sessions;
        // it still yields to the session sitting on its path.
        let cfg = Config::default();
        let rows = vec![
            at("zoxide", "nixos", "/n"),
            at("config", "c", ""),
            at("zellij", "c", "/n"),
        ];
        let out = dedupe(rows, &cfg);
        assert_eq!(sources_paths(&out), vec![("zellij", "/n")]);
    }

    #[test]
    fn blacklist_dirs_by_path_or_glob_never_basename() {
        let names_bl = vec!["scratch".to_string()];
        let dir = at("zoxide", "scratch", "/a/scratch");
        let sess = at("zellij", "scratch", "/a/scratch");
        assert!(!is_blacklisted(&dir, &names_bl, &names_bl));
        assert!(is_blacklisted(&sess, &names_bl, &names_bl));

        let exact = vec!["/a/scratch".to_string()];
        assert!(is_blacklisted(&dir, &exact, &[]));
        assert!(!is_blacklisted(
            &at("zoxide", "scratch", "/b/scratch"),
            &exact,
            &[]
        ));

        let glob = vec!["**/scratch".to_string()];
        assert!(is_blacklisted(&dir, &glob, &[]));
        assert!(is_blacklisted(
            &at("zoxide", "x", "/b/c/scratch"),
            &glob,
            &[]
        ));
        assert!(!is_blacklisted(
            &at("zoxide", "x", "/b/scratch2"),
            &glob,
            &[]
        ));
    }

    #[test]
    fn tsv_shape() {
        let mut r = row("zellij", "foo", false);
        r.path = "/tmp/foo".into();
        r.icon = "▢ ".into();
        assert_eq!(rows_to_tsv(&[r]), "zellij\tfoo\t/tmp/foo\tlive\t▢ foo");
    }
}
