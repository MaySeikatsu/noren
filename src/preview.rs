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

pub fn preview(cfg: &Config, line: &str) {
    let r = parse_line(line);
    if r.state == "sep" {
        println!();
        return;
    }
    let out = match r.state {
        "live" | "exited" => {
            // Visual box-art when configured and running under fzf; text
            // tree otherwise. Both read only the cached layout file — a
            // dead session is never resurrected for a preview.
            // The summary line goes above the render, which itself stays
            // untouched (text mode is golden-tested).
            let summary = summary_line(r.name);
            let reserved = usize::from(summary.is_some());
            let visual = cfg.preview_mode != "text";
            let body = visual
                .then(fzf_preview_dims)
                .flatten()
                .and_then(|(w, h)| layout_preview::render_session_visual(r.name, w, h - reserved))
                .unwrap_or_else(|| layout_preview::render_session(r.name));
            match summary {
                Some(s) => format!("{s}\n{body}"),
                None => body,
            }
        }
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
    println!("{out}");
}

#[cfg(test)]
mod tests {
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
