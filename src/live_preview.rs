//! Live preview: what a RUNNING session shows right now — the focused pane
//! of its active tab, dumped from the server (`zellij action dump-screen`).
//!
//! This is the one preview path that talks to zellij, so it is fenced in:
//! callers only ask for rows the list reported as `live`, every call runs
//! under a kill-timeout (a session that died since the list was drawn would
//! otherwise hang `zellij action` forever), and any failure returns None so
//! the caller falls back to the cached-layout render.

use serde::Deserialize;

use crate::sources::zellij::action_output_with_timeout;
use crate::util::tilde_path;

/// Per zellij call; a healthy server answers in ~20 ms.
const TIMEOUT_MS: u64 = 800;

#[derive(Debug, Deserialize)]
struct Tab {
    position: usize,
    name: String,
    #[serde(default)]
    active: bool,
}

#[derive(Debug, Deserialize)]
struct Pane {
    id: u64,
    #[serde(default)]
    is_plugin: bool,
    #[serde(default)]
    is_focused: bool,
    #[serde(default)]
    is_floating: bool,
    #[serde(default)]
    tab_position: usize,
    #[serde(default)]
    title: String,
    #[serde(default)]
    pane_cwd: Option<String>,
}

fn action_json<T: for<'de> Deserialize<'de>>(name: &str, args: &[&str]) -> Option<T> {
    serde_json::from_str(&action_output_with_timeout(name, args, TIMEOUT_MS)?).ok()
}

/// The tab to show: the one an attached client looks at; for a detached
/// session the focused tab recorded in the resurrection cache; else the
/// first.
fn pick_tab(name: &str, tabs: &[Tab]) -> Option<usize> {
    if let Some(t) = tabs.iter().find(|t| t.active) {
        return Some(t.position);
    }
    let cached = crate::layout_preview::session_focus(name).map(|f| f.tab);
    cached
        .and_then(|n| tabs.iter().find(|t| t.name == n))
        .or_else(|| tabs.iter().min_by_key(|t| t.position))
        .map(|t| t.position)
}

/// The pane to show in a tab: the focused tiled terminal pane. Floating
/// panes only as a last resort — inside the session being previewed, the
/// focused floating pane is usually the picker itself.
fn pick_pane(panes: &[Pane], tab: usize) -> Option<&Pane> {
    let terminals: Vec<&Pane> = panes
        .iter()
        .filter(|p| !p.is_plugin && p.tab_position == tab)
        .collect();
    terminals
        .iter()
        .find(|p| p.is_focused && !p.is_floating)
        .or_else(|| terminals.iter().find(|p| !p.is_floating))
        .or_else(|| terminals.first())
        .copied()
}

/// Text without ANSI escape sequences.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() || e == '~' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn truncate(s: &str, cols: usize) -> String {
    if s.chars().count() <= cols {
        return s.to_string();
    }
    let mut out: String = s.chars().take(cols.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Tab strip: the shown tab in reverse video, the others dim.
fn tab_strip(tabs: &[Tab], shown: usize, cols: usize) -> String {
    let mut sorted: Vec<&Tab> = tabs.iter().collect();
    sorted.sort_by_key(|t| t.position);
    let mut out = String::new();
    let mut width = 0;
    for t in sorted {
        let label = format!(" {} ", truncate(&t.name, 20));
        let w = label.chars().count();
        if width + w > cols {
            out.push_str("\x1b[2m…\x1b[0m");
            break;
        }
        width += w;
        if t.position == shown {
            out.push_str(&format!("\x1b[7m{label}\x1b[0m"));
        } else {
            out.push_str(&format!("\x1b[2m{label}\x1b[0m"));
        }
    }
    out
}

/// The last `rows` non-trailing-blank lines of a screen dump: the newest
/// output sits at the bottom of a terminal, the top is history.
fn tail_lines(screen: &str, rows: usize) -> Vec<&str> {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines
        .iter()
        .rposition(|l| !strip_ansi(l).trim().is_empty())
        .map_or(0, |i| i + 1);
    let start = end.saturating_sub(rows);
    lines[start..end].to_vec()
}

/// Header + tab strip + the pane's current screen, fitted to cols × rows.
pub fn render_live(name: &str, cols: usize, rows: usize) -> Option<String> {
    render_live_at(name, None, cols, rows)
}

/// Live view of one specific tab (0-based `position`; tab menu preview).
pub fn render_live_tab(name: &str, position: usize, cols: usize, rows: usize) -> Option<String> {
    render_live_at(name, Some(position), cols, rows)
}

/// A running session's tabs for the tab menu: 1-based position, name,
/// active flag, focused tiled pane's title · cwd.
pub fn tab_summaries(name: &str) -> Option<Vec<crate::tabs::TabInfo>> {
    let mut tabs: Vec<Tab> = action_json(name, &["list-tabs", "-j", "-s"])?;
    let panes: Vec<Pane> = action_json(name, &["list-panes", "-a", "-j"])?;
    tabs.sort_by_key(|t| t.position);
    Some(
        tabs.into_iter()
            .map(|t| {
                let pane = pick_pane(&panes, t.position);
                crate::tabs::TabInfo {
                    position: t.position + 1,
                    detail: pane.map_or_else(String::new, |p| {
                        crate::tabs::detail(&p.title, p.pane_cwd.as_deref())
                    }),
                    active: t.active,
                    name: t.name,
                }
            })
            .collect(),
    )
}

fn render_live_at(name: &str, want: Option<usize>, cols: usize, rows: usize) -> Option<String> {
    // `zellij action` with an unknown ZELLIJ_SESSION_NAME silently targets
    // whatever session is running (0.45.1) — make sure it's really ours and
    // really live (the list may be seconds old).
    if crate::sources::zellij::session_state(name) != Some("live") {
        return None;
    }
    let tabs: Vec<Tab> = action_json(name, &["list-tabs", "-j", "-s"])?;
    let panes: Vec<Pane> = action_json(name, &["list-panes", "-a", "-j"])?;
    let tab = match want {
        Some(p) if tabs.iter().any(|t| t.position == p) => p,
        _ => pick_tab(name, &tabs)?,
    };
    let pane = pick_pane(&panes, tab)?;
    let screen = action_output_with_timeout(
        name,
        &[
            "dump-screen",
            "-p",
            &format!("terminal_{}", pane.id),
            "--ansi",
        ],
        TIMEOUT_MS,
    )?;

    let tab_name = tabs
        .iter()
        .find(|t| t.position == tab)
        .map_or("", |t| t.name.as_str());
    let cwd = pane.pane_cwd.as_deref().map(tilde_path).unwrap_or_default();
    let mut parts = vec![tab_name, pane.title.as_str()];
    if !cwd.is_empty() {
        parts.push(&cwd);
    }
    // Green dot = live content (the layout render's summary uses ▸).
    let header = truncate(&parts.join(" · "), cols.saturating_sub(2));

    let mut out = vec![format!("\x1b[32m●\x1b[0m \x1b[1m{header}\x1b[0m")];
    if tabs.len() > 1 {
        out.push(tab_strip(&tabs, tab, cols));
    }
    let body_rows = rows.saturating_sub(out.len()).max(1);
    let body = tail_lines(&screen, body_rows);
    if body.is_empty() {
        out.push("\x1b[2m(empty screen)\x1b[0m".to_string());
    } else {
        // Reset per line so a style left open by the dump can't bleed.
        out.extend(body.into_iter().map(|l| format!("{l}\x1b[0m")));
    }
    Some(out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u64, tab: usize, focused: bool, floating: bool, plugin: bool) -> Pane {
        Pane {
            id,
            is_plugin: plugin,
            is_focused: focused,
            is_floating: floating,
            tab_position: tab,
            title: format!("p{id}"),
            pane_cwd: None,
        }
    }

    #[test]
    fn prefers_focused_tiled_pane_over_floating_picker() {
        let panes = vec![
            pane(0, 0, true, false, true), // plugin
            pane(1, 0, false, false, false),
            pane(2, 0, true, false, false),
            pane(3, 0, true, true, false), // floating picker
            pane(4, 1, true, false, false),
        ];
        assert_eq!(pick_pane(&panes, 0).map(|p| p.id), Some(2));
        assert_eq!(pick_pane(&panes, 1).map(|p| p.id), Some(4));
        assert!(pick_pane(&panes, 7).is_none());
        // Only a floating terminal in the tab: show it rather than nothing.
        let only_float = vec![pane(9, 0, true, true, false)];
        assert_eq!(pick_pane(&only_float, 0).map(|p| p.id), Some(9));
    }

    #[test]
    fn active_tab_wins() {
        let tabs = vec![
            Tab {
                position: 0,
                name: "a".into(),
                active: false,
            },
            Tab {
                position: 1,
                name: "b".into(),
                active: true,
            },
        ];
        assert_eq!(pick_tab("no-such-session-xyz", &tabs), Some(1));
    }

    #[test]
    fn detached_without_cache_falls_back_to_first_tab() {
        let tabs = vec![
            Tab {
                position: 1,
                name: "b".into(),
                active: false,
            },
            Tab {
                position: 0,
                name: "a".into(),
                active: false,
            },
        ];
        assert_eq!(pick_tab("no-such-session-xyz", &tabs), Some(0));
    }

    #[test]
    fn tail_keeps_newest_lines_and_drops_trailing_blanks() {
        let screen = "\x1b[mold\n\x1b[m$ ls\n\x1b[ma b\n\x1b[m$ \n\x1b[m\n\x1b[m\n";
        assert_eq!(tail_lines(screen, 2), vec!["\x1b[ma b", "\x1b[m$ "]);
        assert_eq!(tail_lines(screen, 10).len(), 4);
        assert!(tail_lines("\n\n", 5).is_empty());
    }

    #[test]
    fn strip_marks_shown_tab_and_truncates() {
        let tabs = vec![
            Tab {
                position: 0,
                name: "one".into(),
                active: false,
            },
            Tab {
                position: 1,
                name: "two".into(),
                active: true,
            },
        ];
        let s = tab_strip(&tabs, 1, 80);
        assert!(s.contains("\x1b[7m two \x1b[0m"), "{s:?}");
        assert!(s.contains("\x1b[2m one \x1b[0m"), "{s:?}");
        assert!(tab_strip(&tabs, 1, 6).ends_with("…\x1b[0m"));
        assert_eq!(strip_ansi("\x1b[7m two \x1b[0m"), " two ");
    }
}
