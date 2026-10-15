//! Path expansion, name sanitizing, and small shared helpers.

use std::path::{Component, Path, PathBuf};

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Expand `~`, make absolute against the cwd, and normalize `.`/`..`
/// lexically (no symlink resolution) — parity with nushell
/// `path expand --no-symlink` as used throughout zjp2.
pub fn expand_path(p: &str) -> PathBuf {
    let expanded: PathBuf = if p == "~" {
        home_dir()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else {
        PathBuf::from(p)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(expanded)
    };
    let mut out = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Sanitize without the empty-fallback: `[^a-zA-Z0-9._-]` -> `-`, then strip
/// leading `-`. May return an empty string (rename keeps the raw name then).
pub fn sanitize_core(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    replaced.trim_start_matches('-').to_string()
}

/// Session-name sanitizer, parity with zellij-autostart:
///   basename | tr -c 'a-zA-Z0-9._\n-' '-' | sed 's/^-*//'  (empty -> "session")
pub fn sanitize(name: &str) -> String {
    let s = sanitize_core(name);
    if s.is_empty() {
        "session".to_string()
    } else {
        s
    }
}

/// Derive a display name from a path: dir_length = 1 -> basename,
/// 2 -> "parent/basename", etc.
pub fn name_from_path(p: &str, dir_length: usize) -> String {
    let comps: Vec<String> = Path::new(p)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::RootDir => Some("/".to_string()),
            _ => None,
        })
        .collect();
    let n = dir_length.min(comps.len());
    let take = &comps[comps.len() - n..];
    let mut out = String::new();
    for part in take {
        if !out.is_empty() && out != "/" {
            out.push('/');
        }
        out.push_str(part);
    }
    out
}

/// Abbreviate a `$HOME` prefix to `~` for display (only on a component
/// boundary: `/home/u2` stays as-is for HOME=/home/u).
pub fn tilde_path(p: &str) -> String {
    let home = home_dir().to_string_lossy().into_owned();
    let home = home.trim_end_matches('/');
    if home.is_empty() {
        return p.to_string();
    }
    match p.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => p.to_string(),
    }
}

/// POSIX single-quote a string for `sh` (`'` -> `'\''`).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Does the argument look like a path (vs a session name)?
pub fn is_pathlike(s: &str) -> bool {
    s.starts_with('/') || s.starts_with('~') || s.starts_with("./") || s.starts_with("../")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_and_strips() {
        assert_eq!(sanitize("Hello World!"), "Hello-World-");
        assert_eq!(sanitize("---x"), "x");
        assert_eq!(sanitize("a.b_c-d"), "a.b_c-d");
        assert_eq!(sanitize("!!!"), "session");
        assert_eq!(sanitize(""), "session");
        assert_eq!(sanitize("übung"), "-bung".trim_start_matches('-'));
    }

    #[test]
    fn name_from_path_lengths() {
        assert_eq!(name_from_path("/home/maike/nixos", 1), "nixos");
        assert_eq!(name_from_path("/home/maike/nixos", 2), "maike/nixos");
        assert_eq!(name_from_path("/home/maike/nixos", 9), "/home/maike/nixos");
        assert_eq!(name_from_path("nixos", 1), "nixos");
    }

    #[test]
    fn expand_tilde() {
        let home = home_dir();
        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/x"), home.join("x"));
        assert_eq!(expand_path("/a/b/../c/./d"), PathBuf::from("/a/c/d"));
    }

    #[test]
    fn tilde_abbreviates_home_on_component_boundary() {
        let home = home_dir().to_string_lossy().into_owned();
        assert_eq!(tilde_path(&home), "~");
        assert_eq!(tilde_path(&format!("{home}/a/b")), "~/a/b");
        assert_eq!(tilde_path(&format!("{home}x/a")), format!("{home}x/a"));
        assert_eq!(tilde_path("/elsewhere"), "/elsewhere");
    }

    #[test]
    fn shell_quote_single_quotes() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn pathlike() {
        assert!(is_pathlike("/x"));
        assert!(is_pathlike("~/x"));
        assert!(is_pathlike("./x"));
        assert!(is_pathlike("../x"));
        assert!(!is_pathlike("nixos"));
    }
}
