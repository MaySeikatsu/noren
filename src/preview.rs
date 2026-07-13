//! fzf preview: receives the whole TSV row and dispatches on state.
//!   live/exited -> native layout tree render
//!   config      -> the entry's preview_command ({} = path)
//!   dir/other   -> wildcard preview_command, else eza tree, else ls -la

use std::process::Command;

use crate::config::Config;
use crate::layout_preview;

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

fn render_path(path: &str, preview_cmd: &str) -> String {
    if !preview_cmd.is_empty() {
        let cmd = preview_cmd.replace("{}", path);
        let mut c = Command::new("sh");
        c.args(["-c", &cmd]);
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
            let visual = cfg.preview_mode != "text";
            visual
                .then(fzf_preview_dims)
                .flatten()
                .and_then(|(w, h)| layout_preview::render_session_visual(r.name, w, h))
                .unwrap_or_else(|| layout_preview::render_session(r.name))
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
