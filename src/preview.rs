//! fzf preview: receives the whole TSV row and dispatches on state.
//!   live/exited -> focus summary line + native layout render
//!   config      -> the entry's preview_command ({} = shell-quoted path,
//!                  also exported as $NOREN_PATH)
//!   dir/other   -> wildcard preview_command, else eza tree, else ls -la

use std::process::Command;

use crate::config::Config;
use crate::layout_preview;
use crate::util::{shell_quote, tilde_path};

struct ParsedLine<'a> {
    name: &'a str,
    path: &'a str,
    state: &'a str,
}

/// First 4 tab-delimited fields; robust to icon prefixes in the display
/// column (never parsed).
fn parse_line(line: &str) -> ParsedLine<'_> {
    let mut parts = line.split('\t');
    let _source = parts.next().unwrap_or("");
    ParsedLine {
        name: parts.next().unwrap_or(""),
        path: parts.next().unwrap_or(""),
        state: parts.next().unwrap_or(""),
    }
}

fn run_capture(mut cmd: Command) -> Option<String> {
    cmd.output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Substitute the path for `{}` as exactly one shell-quoted word. Configs
/// may already quote the placeholder (`'{}'`, `"{}"`), so those forms are
/// replaced whole. Single left-to-right pass: a `{}` inside the path is
/// never substituted again.
fn substitute_path(preview_cmd: &str, path: &str) -> String {
    let quoted = shell_quote(path);
    let mut out = String::with_capacity(preview_cmd.len() + quoted.len());
    let mut rest = preview_cmd;
    while !rest.is_empty() {
        if let Some(r) = ["'{}'", "\"{}\"", "{}"]
            .iter()
            .find_map(|form| rest.strip_prefix(form))
        {
            out.push_str(&quoted);
            rest = r;
        } else {
            let c = rest.chars().next().expect("non-empty");
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

fn render_path(path: &str, preview_cmd: &str) -> String {
    if !preview_cmd.is_empty() {
        let cmd = substitute_path(preview_cmd, path);
        let mut c = Command::new("sh");
        c.args(["-c", &cmd]).env("NOREN_PATH", path);
        run_capture(c).unwrap_or_else(|| format!("(preview command failed: {cmd})"))
    } else if !path.is_empty() && std::path::Path::new(path).exists() {
        let mut eza = Command::new("eza");
        eza.args(["--tree", "--level=2", "--color=always", path]);
        run_capture(eza).unwrap_or_else(|| {
            let mut ls = Command::new("ls");
            ls.args(["-la", path]);
            run_capture(ls).unwrap_or_default()
        })
    } else {
        "(no path)".to_string()
    }
}

/// Expand tabs to 8-column stops. The picker runs fzf with `--tabstop 1`
/// (its name/path columns are tab-joined), which would otherwise squash
/// tabs in preview_command output. ANSI escapes take no columns.
fn expand_tabs(s: &str) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\t' => {
                let n = 8 - col % 8;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            }
            '\n' => {
                out.push(c);
                col = 0;
            }
            '\x1b' => {
                // CSI: ESC [ params final-byte — zero width.
                out.push(c);
                if chars.peek() == Some(&'[') {
                    for e in chars.by_ref() {
                        out.push(e);
                        if e.is_ascii_alphabetic() || e == '~' {
                            break;
                        }
                    }
                }
            }
            _ => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

/// fzf exports the preview pane size; visual mode needs it to scale boxes.
fn fzf_preview_dims() -> Option<(usize, usize)> {
    let w: usize = std::env::var("FZF_PREVIEW_COLUMNS").ok()?.parse().ok()?;
    let h: usize = std::env::var("FZF_PREVIEW_LINES").ok()?.parse().ok()?;
    if w >= 24 && h >= 8 {
        Some((w, h))
    } else {
        None
    }
}

/// One line above a session preview: what was last active there, e.g.
/// `▸ code · hx · ~/Projects/noren`. Plain text, truncated to `cols`.
fn format_summary(f: &layout_preview::FocusSummary, cols: Option<usize>) -> String {
    let mut parts = vec![format!("▸ {}", f.tab)];
    match f.command.as_deref() {
        Some("") => parts.push("shell".into()),
        Some(cmd) => parts.push(cmd.into()),
        None => {}
    }
    if !f.cwd.is_empty() {
        parts.push(tilde_path(&f.cwd));
    }
    let line = parts.join(" · ").replace(['\n', '\r'], " ");
    match cols {
        Some(c) if line.chars().count() > c => {
            let mut cut: String = line.chars().take(c.saturating_sub(1)).collect();
            cut.push('…');
            cut
        }
        _ => line,
    }
}

fn summary_line(name: &str) -> Option<String> {
    let f = layout_preview::session_focus(name)?;
    let cols = std::env::var("FZF_PREVIEW_COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok());
    // Styled after truncation so an escape sequence is never cut.
    Some(format!("\x1b[1m{}\x1b[0m", format_summary(&f, cols)))
}

/// Session preview from the cached layout only: visual box-art (unless
/// preview_mode = "text") under fzf, else the text tree, with the "last
/// active" summary line on top. Never talks to zellij.
fn layout_render(cfg: &Config, name: &str) -> String {
    // The render itself stays untouched (text mode is golden-tested).
    let summary = summary_line(name);
    let reserved = usize::from(summary.is_some());
    let visual = cfg.preview_mode != "text";
    let body = visual
        .then(fzf_preview_dims)
        .flatten()
        .and_then(|(w, h)| layout_preview::render_session_visual(name, w, h - reserved))
        .unwrap_or_else(|| layout_preview::render_session(name));
    match summary {
        Some(s) => format!("{s}\n{body}"),
        None => body,
    }
}

/// `mode` overrides `preview_mode` for this call (the picker's live/layout
/// toggle): "live" | "layout".
pub fn preview(cfg: &Config, line: &str, mode: Option<&str>) {
    let r = parse_line(line);
    if r.state == "sep" {
        println!();
        return;
    }
    let live = mode.unwrap_or(&cfg.preview_mode) == "live";
    let out = match r.state {
        // Live mode: the session's current screen. Only for rows listed as
        // running — an exited session is never contacted (it would hang or
        // resurrect); failures fall back to the layout render below.
        "live" if live => fzf_preview_dims()
            .and_then(|(w, h)| crate::live_preview::render_live(r.name, w, h))
            .unwrap_or_else(|| layout_render(cfg, r.name)),
        "live" | "exited" => layout_render(cfg, r.name),
        // Snapshot menu: what a snapshot holds (the file only), and for the
        // "take one now" row what the session's cache looks like.
        "snapshot" => {
            let file = std::path::Path::new(r.path);
            let head = format!("\x1b[1m📸 {}\x1b[0m", crate::snapshot::describe(file));
            let body = (cfg.preview_mode != "text")
                .then(fzf_preview_dims)
                .flatten()
                .and_then(|(w, h)| layout_preview::render_file_visual(file, w, h - 1))
                .unwrap_or_else(|| layout_preview::render_file(file));
            format!("{head}\n{body}")
        }
        "snapnew" => layout_render(cfg, r.name),
        // Tab menu: that tab's live screen when the session runs (path
        // holds the 1-based position), else the saved layout.
        "tab" => {
            let position: usize = r.path.parse().unwrap_or(1);
            fzf_preview_dims()
                .filter(|_| cfg.preview_mode == "live")
                .and_then(|(w, h)| {
                    crate::live_preview::render_live_tab(r.name, position.saturating_sub(1), w, h)
                })
                .unwrap_or_else(|| layout_render(cfg, r.name))
        }
        "tabnone" => String::new(),
        "config" => {
            let pv = cfg
                .find_session(r.name)
                .and_then(|s| s.preview_command.clone())
                .unwrap_or_default();
            render_path(r.path, &pv)
        }
        _ => {
            let pv = if r.path.is_empty() {
                String::new()
            } else {
                cfg.match_wildcard(r.path)
                    .and_then(|w| w.preview_command.clone())
                    .unwrap_or_default()
            };
            render_path(r.path, &pv)
        }
    };
    println!("{}", expand_tabs(&out));
}

#[cfg(test)]
mod tests {
    #[test]
    fn tabs_expand_to_eight_column_stops_ignoring_ansi() {
        assert_eq!(super::expand_tabs("a\tb"), "a       b");
        assert_eq!(
            super::expand_tabs("12345678\tx\n\ty"),
            "12345678        x\n        y"
        );
        assert_eq!(
            super::expand_tabs("\x1b[1mab\x1b[0m\tc"),
            "\x1b[1mab\x1b[0m      c"
        );
        assert_eq!(super::expand_tabs("no tabs"), "no tabs");
    }

    use super::*;

    #[test]
    fn placeholder_forms_all_become_one_quoted_word() {
        let p = "/x/a b'c";
        let q = "'/x/a b'\\''c'";
        for cmd in ["ls {}", "ls '{}'", "ls \"{}\""] {
            assert_eq!(substitute_path(cmd, p), format!("ls {q}"), "cmd: {cmd}");
        }
        assert_eq!(substitute_path("diff {} {}", "/a"), "diff '/a' '/a'");
        assert_eq!(substitute_path("echo hi", "/a"), "echo hi");
    }

    #[test]
    fn summary_formats_and_truncates() {
        use layout_preview::FocusSummary;
        let f = FocusSummary {
            tab: "code".into(),
            command: Some("hx".into()),
            cwd: "/srv/noren".into(),
        };
        assert_eq!(format_summary(&f, None), "▸ code · hx · /srv/noren");
        assert_eq!(format_summary(&f, Some(10)), "▸ code · …");
        let shell = FocusSummary {
            command: Some(String::new()),
            ..f
        };
        assert_eq!(format_summary(&shell, None), "▸ code · shell · /srv/noren");
        let bare = FocusSummary {
            tab: "t".into(),
            command: None,
            cwd: String::new(),
        };
        assert_eq!(format_summary(&bare, None), "▸ t");
    }

    #[test]
    fn braces_inside_the_path_are_not_substituted_again() {
        assert_eq!(substitute_path("ls {}", "/x/{}"), "ls '/x/{}'");
        assert_eq!(substitute_path("ls ü{}", "/é"), "ls ü'/é'");
    }
}
