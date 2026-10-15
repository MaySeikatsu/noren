//! Interactive fzf picker.
//!
//! fzf is the UI; noren feeds it TSV rows and re-execs itself for reloads,
//! previews, and actions. Row actions (pin, snapshot, kill, delete, enter,
//! esc) go through `noren pick <event>` as fzf `transform` handlers: they
//! do the work in-process and answer with fzf actions on stdout — reload
//! the current view, show a status line in the ghost text, or switch the
//! prompt into an inline y/N confirmation.
//!
//! Keybinds come from `[keys]`; the header is generated from them, so it
//! always lists exactly the active binds.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::Context;

use crate::actions::{self, Kind, SessionState};
use crate::config::Config;
use crate::connect::session_connect;
use crate::sources::{Selection, collect, rows_to_tsv};

/// Views: (keys action, prompt, `noren list` source word).
const VIEWS: &[(&str, &str, &str)] = &[
    ("sessions", "▢  ", "zellij"),
    ("dirs", "📁  ", "zoxide"),
    ("config", "⚙  ", "config"),
    ("all", "⚡  ", "all"),
];
const DEFAULT_PROMPT: &str = "⚡  ";
const HINT: &str = "type to filter · ⏎ on no match creates a session";
/// Snapshot menu: prompt is "📸 <session>  "; its rows come from
/// `noren snapshot-rows <session>`.
const SNAP_PREFIX: &str = "📸 ";
const SNAP_HINT: &str = "⏎ restore · ^s snapshot now · ^d delete snapshot · esc back";
/// Tab menu: prompt is "▤ <session>  "; rows from `noren tab-rows`.
const TAB_PREFIX: &str = "▤ ";
const TAB_HINT: &str = "⏎ go to this tab · esc back";
/// stdout markers for work done after fzf exits (it may exec zellij).
const RESTORE_MARK: &str = "__noren_restore__\t";
const MKDIR_MARK: &str = "__noren_mkdir__\t";
const TAB_MARK: &str = "__noren_tab__\t";

/// The binary fzf binds re-exec. current_exe keeps the picker working when
/// noren isn't on PATH (cargo target dir, nix result/).
fn self_exe() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "noren".to_string())
}

/// `noren list` source for a prompt; unknown prompts show everything.
fn view_source(prompt: &str) -> &'static str {
    VIEWS
        .iter()
        .find(|(_, p, _)| *p == prompt)
        .map(|(_, _, s)| *s)
        .unwrap_or("all")
}

/// "ctrl-t" -> "^t", "alt-x" -> "M-x", "ctrl-alt-d" -> "^M-d",
/// "alt-enter" -> "M-⏎".
fn pretty_key(key: &str) -> String {
    let mut k = key.trim().to_string();
    for (from, to) in [("ctrl-", "^"), ("alt-", "M-"), ("enter", "⏎")] {
        k = k.replace(from, to);
    }
    k
}

/// One key-overview entry: action, label, priority (lower = kept longer
/// when the compact overview runs out of room).
type Entry = (&'static str, &'static str, u8);

/// Compact overview, line 1: views and list controls.
const COMPACT_VIEWS: &[Entry] = &[
    ("sessions", "sessions", 2),
    ("dirs", "dirs", 3),
    ("config", "config", 9),
    ("all", "all", 4),
    ("reload", "reload", 8),
    ("preview", "preview", 5),
];
/// Compact overview, line 2: row actions, then ⏎ / esc.
const COMPACT_ACTIONS: &[Entry] = &[
    ("tabs", "tabs", 6),
    ("pin", "pin", 10),
    ("rename", "rename", 9),
    ("kill", "kill", 7),
    ("delete", "delete", 8),
    ("enter", "connect", 1),
    ("esc", "close", 3),
];
/// Everything else, in the full overview (F1) — which lists EVERY bind.
const FULL_MORE: &[Entry] = &[
    ("preview_style", "preview style", 0),
    ("new_window", "new window", 0),
    ("snapshot", "snapshots", 0),
    ("hide", "hide", 0),
    ("hidden", "show hidden", 0),
    ("paths", "search paths", 0),
    ("live", "preview (alias)", 0),
];

/// Key overview above the prompt. Compact (default): at most
/// `header_lines` lines of the most used keys — entries that don't fit
/// are dropped by priority — with "F1 all keys" in the bottom-right
/// corner. Full: every bind, wrapped to `width`. Config warnings go on top.
fn header(cfg: &Config, warnings: &[String], width: Option<usize>, full: bool) -> String {
    let binds = effective_binds(cfg);
    let key_of = |action: &str| -> Option<String> {
        match action {
            "enter" => Some("⏎".to_string()),
            "esc" => Some("esc".to_string()),
            _ => binds
                .iter()
                .find(|(a, _)| *a == action)
                .map(|(_, k)| pretty_key(k)),
        }
    };
    // The config view is empty without [[session]] entries — don't
    // advertise it.
    // The full overview lists every bind; the compact one doesn't
    // advertise an empty config view.
    let wanted = |action: &str| full || action != "config" || !cfg.session.is_empty();
    let cell = |key: &str, label: &str| {
        (
            key.chars().count() + 1 + label.chars().count(),
            format!("\x1b[1m{key}\x1b[0m \x1b[2m{label}\x1b[0m"),
        )
    };
    let entries = |list: &[Entry]| -> Vec<(u8, usize, String)> {
        list.iter()
            .filter(|(a, _, _)| wanted(a))
            .filter_map(|(a, label, prio)| {
                key_of(a).map(|k| {
                    let (w, text) = cell(&k, label);
                    (*prio, w, text)
                })
            })
            .collect()
    };
    let help_key = key_of("help");
    let max = width.unwrap_or(usize::MAX).max(10);

    let mut out: Vec<String> = warnings
        .iter()
        .map(|w| format!("\x1b[33m⚠ {w}\x1b[0m"))
        .collect();

    if full {
        let mut all = entries(COMPACT_VIEWS);
        all.extend(entries(FULL_MORE));
        all.extend(entries(COMPACT_ACTIONS));
        if let Some(k) = &help_key {
            let (w, t) = cell(k, "less");
            all.push((0, w, t));
        }
        out.extend(wrap_cells(&all, max));
        return out.join("\n");
    }

    let lines = cfg.header_lines.max(1);
    let help = help_key.map(|k| cell(&k, "more"));
    let mut groups = vec![entries(COMPACT_VIEWS), entries(COMPACT_ACTIONS)];
    if lines == 1 {
        let merged = groups.concat();
        groups = vec![merged];
    }
    // Lines per group: views get the smaller half.
    let per_group: Vec<usize> = if groups.len() == 1 {
        vec![1]
    } else {
        vec![lines / 2, lines - lines / 2]
    };
    // The F1 hint goes to the bottom-right corner — or, if that would cost
    // more keys than ending the first line with it, there.
    let hint_w = help.as_ref().map_or(0, |(w, _)| w + 2);
    let fit_group = |mut cells: Vec<(u8, usize, String)>, budget: usize, reserve: usize| {
        // Drop the lowest-priority entries until the group fits.
        let mut dropped = 0;
        while !cells.is_empty() && !fits(&cells, max, budget, reserve) {
            let worst = cells
                .iter()
                .enumerate()
                .max_by_key(|(_, (p, _, _))| *p)
                .map(|(i, _)| i)
                .expect("non-empty");
            cells.remove(worst);
            dropped += 1;
        }
        (cells, dropped)
    };
    let last = groups.len() - 1;
    let place = |hint_group: usize| {
        let fitted: Vec<_> = groups
            .iter()
            .enumerate()
            .map(|(gi, cells)| {
                let reserve = if gi == hint_group { hint_w } else { 0 };
                fit_group(cells.clone(), per_group[gi], reserve)
            })
            .collect();
        let dropped: usize = fitted.iter().map(|(_, d)| d).sum();
        (fitted, dropped)
    };
    let (bottom, bottom_drops) = place(last);
    let (fitted, hint_group) = if last > 0 && bottom_drops > 0 {
        let (top, top_drops) = place(0);
        if top_drops < bottom_drops {
            (top, 0)
        } else {
            (bottom, last)
        }
    } else {
        (bottom, last)
    };
    for (gi, (cells, _)) in fitted.into_iter().enumerate() {
        let mut wrapped = wrap_cells(&cells, max);
        if gi == hint_group
            && let Some((hw, ht)) = &help
        {
            let mut last = wrapped.pop().unwrap_or_default();
            // No room left on the last line: the hint gets its own (fits()
            // only allows that when the budget has a spare line).
            if !last.is_empty() && visible_width(&last) + 2 + hw > max {
                wrapped.push(last);
                last = String::new();
            }
            let used = visible_width(&last);
            let gap = if width.is_some() {
                max.saturating_sub(used + hw)
                    .max(if used == 0 { 0 } else { 2 })
            } else if used == 0 {
                0
            } else {
                2
            };
            wrapped.push(format!("{last}{:gap$}{ht}", ""));
        }
        out.extend(wrapped);
    }
    out.join("\n")
}

/// Greedy wrap of (prio, width, text) cells into lines of `max` columns.
fn wrap_cells(cells: &[(u8, usize, String)], max: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let (mut cur, mut cur_w) = (String::new(), 0usize);
    for (_, w, text) in cells {
        if cur_w > 0 && cur_w + 2 + w > max {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if cur_w > 0 {
            cur.push_str("  ");
            cur_w += 2;
        }
        cur.push_str(text);
        cur_w += w;
    }
    if cur_w > 0 {
        lines.push(cur);
    }
    lines
}

/// Do the cells wrap into `budget` lines, leaving `reserve` columns free
/// at the end of the last one?
fn fits(cells: &[(u8, usize, String)], max: usize, budget: usize, reserve: usize) -> bool {
    let mut lines = 1usize;
    let mut cur = 0usize;
    for (_, w, _) in cells {
        let need = if cur == 0 { *w } else { cur + 2 + w };
        if need > max {
            lines += 1;
            cur = *w;
        } else {
            cur = need;
        }
    }
    lines < budget || (lines == budget && cur + reserve <= max)
}

/// Columns a styled header line occupies.
fn visible_width(s: &str) -> usize {
    let mut n = 0;
    let mut esc = false;
    for c in s.chars() {
        match (esc, c) {
            (false, '\x1b') => esc = true,
            (true, c) if c.is_ascii_alphabetic() => esc = false,
            (false, _) => n += 1,
            _ => {}
        }
    }
    n
}

/// Columns the header gets: the list pane, i.e. the terminal minus a
/// side-by-side preview (same switch as Config::preview_window: a side
/// preview takes 60% unless that would be under 56 columns).
fn header_width(cfg: &Config, cols: usize) -> usize {
    let side = !matches!(cfg.preview_position.as_str(), "up" | "down");
    let preview = cols * 60 / 100;
    let list = if side && preview >= 56 {
        cols - preview
    } else {
        cols
    };
    list.saturating_sub(3)
}

/// Active binds with duplicates resolved like fzf does (last one wins).
fn effective_binds(cfg: &Config) -> Vec<(&'static str, String)> {
    let active = cfg.keys.active();
    active
        .iter()
        .enumerate()
        .filter(|(i, (_, k))| !active[i + 1..].iter().any(|(_, k2)| k2 == k))
        .filter(|(_, (_, k))| !matches!(*k, "enter" | "esc" | "change" | "start"))
        .map(|(_, (a, k))| (*a, k.trim().to_string()))
        .collect()
}

fn fzf_binds(cfg: &Config, exe: &str) -> Vec<String> {
    let mut binds = Vec::new();
    for (action, key) in effective_binds(cfg) {
        let body = match action {
            "reload" => format!("transform({exe} pick reload)"),
            "pin" | "snapshot" | "kill" | "delete" => {
                format!("transform({exe} pick {action} {{}})")
            }
            // One key cycles the preview: live → layout → text → off.
            // combined: one key cycles styles and off; separate: it only
            // shows/hides and preview_style cycles the style.
            "preview" | "live" if cfg.preview_toggle == "separate" => "toggle-preview".to_string(),
            "preview" | "live" => format!("transform({exe} pick preview-mode)"),
            "preview_style" => format!("transform({exe} pick preview-style)"),
            "hide" => format!("transform({exe} pick hide {{}})"),
            "rename" => format!("transform({exe} pick rename {{}})"),
            "tabs" => format!("transform({exe} pick tabs {{}})"),
            "paths" => format!("transform({exe} pick paths)"),
            "help" => format!("transform({exe} pick help)"),
            "new_window" => format!("transform({exe} pick new-window {{}})"),
            "hidden" => format!("transform({exe} pick show-hidden)"),
            view => {
                let (_, prompt, src) = VIEWS
                    .iter()
                    .find(|(a, _, _)| *a == view)
                    .expect("every non-row action is a view");
                format!("change-prompt({prompt})+reload({exe} list {src})")
            }
        };
        binds.push(format!("{key}:{body}"));
    }
    binds.push(format!("enter:transform({exe} pick enter {{}})"));
    binds.push(format!("esc:transform({exe} pick esc)"));
    // Only live while a y/N confirmation is pending.
    binds.push(format!("change:transform({exe} pick answer)"));
    binds.push(format!(
        "start:unbind(change)+transform-header({exe} pick header)"
    ));
    binds.push(format!("resize:transform-header({exe} pick header)"));
    binds
}

pub fn picker() -> anyhow::Result<()> {
    let (cfg, warnings) = Config::load_with_warnings();
    let rows = collect(&cfg, Selection::all());
    let tsv = rows_to_tsv(&rows, &cfg);
    if tsv.is_empty() {
        for w in &warnings {
            eprintln!("noren: {w}");
        }
        eprintln!("noren: no sessions, no zoxide dirs, no config entries");
        return Ok(());
    }

    let exe = crate::util::shell_quote(&self_exe());
    let id = std::process::id().to_string();
    let mut cmd = Command::new("fzf");
    cmd.args([
        "--ansi",
        "--tiebreak=index",
        // Keep the cursor on the same entry across reloads (auto-reload,
        // pin, kill, …): identity = source, name, path.
        "--track",
        "--id-nth",
        "1..3",
        "--delimiter",
        "\t",
        // Name (5) and dimmed path (6) are shown; only the name is matched.
        "--with-nth",
        "5,6",
        "--nth",
        "1",
        "--tabstop",
        "1",
        "--prompt",
        DEFAULT_PROMPT,
        "--ghost",
        HINT,
        "--header",
        &header(&cfg, &warnings, None, false),
        // Binds are written for POSIX sh, whatever the user's $SHELL is.
        "--with-shell",
        "sh -c",
        "--preview",
        &format!("{exe} preview {{}}"),
        // The preview replaces this with what it actually shows.
        "--preview-label",
        " preview ",
        "--preview-window",
        &cfg.preview_window(),
    ]);
    for b in fzf_binds(&cfg, &exe) {
        cmd.arg("--bind").arg(b);
    }
    // Picker and preview as two areas: the preview a shade off the
    // terminal's background (or preview_bg as given).
    let preview_bg = if !cfg.preview_bg.trim().is_empty() {
        Some(cfg.preview_bg.trim().to_string())
    } else if cfg.preview_tint {
        crate::tint::terminal_bg().map(crate::tint::shade)
    } else {
        None
    };
    if let Some(bg) = preview_bg {
        cmd.arg("--color").arg(format!("preview-bg:{bg}"));
    }
    // fzf's HTTP API: previews set their own label through it, and live
    // previews refresh on a timer. Unix socket paths max out at ~107 bytes;
    // go without rather than fail to start fzf in an exotic runtime dir.
    let sock = Some(fzf_socket(&id)).filter(|p| p.as_os_str().len() < 100);
    if let Some(sock) = &sock {
        cmd.arg("--listen").arg(sock);
    }
    let mut child = cmd
        // Re-execs show config problems in the header, not on stderr.
        .env("NOREN_QUIET", "1")
        .env("NOREN_PICKER_ID", &id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("noren: failed to spawn fzf (is it on PATH?)")?;

    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(format!("{tsv}\n").as_bytes())
        .ok();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refresher = sock
        .clone()
        .filter(|_| cfg.live_refresh_ms > 0 || cfg.auto_reload_secs > 0)
        .map(|sock| {
            let stop = stop.clone();
            let ms = std::time::Duration::from_millis;
            let live = (cfg.live_refresh_ms > 0).then(|| ms(cfg.live_refresh_ms.max(200)));
            let list = (cfg.auto_reload_secs > 0).then(|| ms(cfg.auto_reload_secs.max(5) * 1000));
            let exe = exe.clone();
            std::thread::spawn(move || refresh_loop(&sock, &exe, live, list, &stop))
        });
    let out = child.wait_with_output()?;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(t) = refresher {
        let _ = t.join();
    }
    if let Some(sock) = &sock {
        let _ = std::fs::remove_file(sock);
    }
    let _ = std::fs::remove_file(pending_file(&id));
    let _ = std::fs::remove_file(show_hidden_file(&id));
    let _ = std::fs::remove_file(mode_file(&id));
    let _ = std::fs::remove_file(keys_file(&id));

    // Non-zero = esc / ctrl-c abort; just close the picker.
    if !out.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let result = if let Some(rest) = stdout.lines().find_map(|l| l.strip_prefix(RESTORE_MARK)) {
        let (name, file) = rest.split_once('\t').unwrap_or((rest, ""));
        crate::snapshot::restore_snapshot(&cfg, name, std::path::Path::new(file), true)
    } else if let Some(rest) = stdout.lines().find_map(|l| l.strip_prefix(TAB_MARK)) {
        let (session, pos) = rest.split_once('\t').unwrap_or((rest, "1"));
        crate::tabs::connect_tab(&cfg, session, pos.parse().unwrap_or(1))
    } else if let Some(dir) = stdout.lines().find_map(|l| l.strip_prefix(MKDIR_MARK)) {
        std::fs::create_dir_all(dir)
            .map_err(anyhow::Error::from)
            .and_then(|()| session_connect(&cfg, dir))
    } else {
        let Some(target) = target_for(stdout.trim()) else {
            return Ok(());
        };
        session_connect(&cfg, &target)
    };
    if let Err(e) = &result
        && crate::connect::inside_zellij()
        && std::io::stdin().is_terminal()
    {
        // The picker usually runs in a floating pane that closes on exit;
        // hold it open so the error can actually be read.
        eprintln!("noren: {e:#}");
        eprint!("\npress enter to close ");
        let _ = std::io::stdin().read_line(&mut String::new());
        return Ok(());
    }
    result
}

/// What fzf printed -> connect target. A full row connects sessions by
/// NAME (their path is just the first tab's cwd and would resolve to a
/// different session) and dirs/config entries by path. Anything without
/// tabs is the query, printed by accept-or-print-query on no match.
fn target_for(line: &str) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    if !line.contains('\t') {
        return Some(line.to_string());
    }
    let mut parts = line.split('\t');
    let source = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let target = match source {
        "sep" => "",
        "zellij" => name,
        _ if !path.is_empty() && std::path::Path::new(path).exists() => path,
        _ => name,
    };
    (!target.is_empty()).then(|| target.to_string())
}

// ---- live preview refresh ----------------------------------------------------

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// fzf's --listen socket (must end in .sock; unix socket paths are short).
fn fzf_socket(id: &str) -> PathBuf {
    runtime_dir().join(format!("noren-fzf-{id}.sock"))
}

/// Send fzf actions from a child process (preview, transform) through the
/// socket fzf exports as $FZF_SOCK. No-op outside the picker.
pub(crate) fn fzf_post(actions: &str) {
    if let Some(sock) = std::env::var_os("FZF_SOCK") {
        let _ = fzf_http(std::path::Path::new(&sock), "POST", actions);
    }
}

/// One HTTP/1.0 exchange with fzf's --listen server; the response body.
fn fzf_http(sock: &std::path::Path, method: &str, body: &str) -> Option<String> {
    use std::io::Read;
    let mut s = std::os::unix::net::UnixStream::connect(sock).ok()?;
    s.set_read_timeout(Some(std::time::Duration::from_millis(500)))
        .ok()?;
    let key = std::env::var("FZF_API_KEY")
        .map(|k| format!("x-api-key: {k}\r\n"))
        .unwrap_or_default();
    // fzf answers 400 to a Content-Length: 0 GET, so only send it with a body.
    let len = if body.is_empty() {
        String::new()
    } else {
        format!("Content-Length: {}\r\n", body.len())
    };
    // fzf only accepts the query string on GET ("POST /" or 400).
    let path = if method == "GET" { "/?limit=0" } else { "/" };
    let req = format!("{method} {path} HTTP/1.0\r\nHost: localhost\r\n{key}{len}\r\n{body}");
    s.write_all(req.as_bytes()).ok()?;
    let mut resp = String::new();
    let _ = s.read_to_string(&mut resp);
    resp.split_once("\r\n\r\n").map(|(_, b)| b.to_string())
}

/// Is fzf's current row a live session? (GET state, `current.text`.)
fn current_row_is_live(state: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(state)
        .ok()
        .and_then(|v| v["current"]["text"].as_str().map(str::to_string))
        .is_some_and(|t| t.split('\t').nth(3) == Some("live"))
}

/// While the picker runs:
/// - every `live`: re-render the preview if the cursor sits on a live
///   session (other previews don't change on their own, and re-running
///   e.g. eza would only flicker);
/// - every `list`: re-list the current view, so sessions started or ended
///   elsewhere don't show stale states (`pick auto-reload`, which skips
///   while a confirmation or rename is pending).
fn refresh_loop(
    sock: &std::path::Path,
    exe: &str,
    live: Option<std::time::Duration>,
    list: Option<std::time::Duration>,
    stop: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    let tick = std::time::Duration::from_millis(50);
    let (mut live_wait, mut list_wait) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(tick);
        live_wait += tick;
        list_wait += tick;
        if let Some(every) = list
            && list_wait >= every
        {
            list_wait = std::time::Duration::ZERO;
            let _ = fzf_http(
                sock,
                "POST",
                &act("transform", &format!("{exe} pick auto-reload")),
            );
        }
        if let Some(every) = live
            && live_wait >= every
        {
            live_wait = std::time::Duration::ZERO;
            if fzf_http(sock, "GET", "").is_some_and(|st| current_row_is_live(&st)) {
                let _ = fzf_http(sock, "POST", "refresh-preview");
            }
        }
    }
}

// ---- `noren pick <event>`: fzf transform handlers -------------------------

/// An fzf action with an argument, using a delimiter pair the argument
/// doesn't contain (names/messages may contain parentheses).
pub(crate) fn act(name: &str, arg: &str) -> String {
    for (open, close) in [
        ('(', ')'),
        ('[', ']'),
        ('{', '}'),
        ('<', '>'),
        ('~', '~'),
        ('!', '!'),
    ] {
        if !arg.contains(close) {
            return format!("{name}{open}{arg}{close}");
        }
    }
    format!("{name}:{arg}")
}

fn pending_file(id: &str) -> PathBuf {
    runtime_dir().join(format!("noren-pick-{id}"))
}

/// What a pending confirmation will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Kill,
    Delete,
    /// Recreate the session from the snapshot file in `Pending::file`.
    Restore,
    /// Delete the snapshot file in `Pending::file`.
    DropSnapshot,
    /// Rename the session to the query (⏎ commits; not a y/N).
    Rename,
    /// Create the folder in `Pending::file` and a session in it.
    Mkdir,
}

impl Action {
    fn word(self) -> &'static str {
        match self {
            Action::Kill => "kill",
            Action::Delete => "delete",
            Action::Restore => "restore",
            Action::DropSnapshot => "drop",
            Action::Rename => "rename",
            Action::Mkdir => "mkdir",
        }
    }

    fn from_word(w: &str) -> Option<Action> {
        [
            Action::Kill,
            Action::Delete,
            Action::Restore,
            Action::DropSnapshot,
            Action::Rename,
            Action::Mkdir,
        ]
        .into_iter()
        .find(|a| a.word() == w)
    }

    fn cancelled(self) -> &'static str {
        match self {
            Action::Kill => "kill cancelled",
            Action::Delete => "delete cancelled",
            Action::Restore => "restore cancelled",
            Action::DropSnapshot => "snapshot kept",
            Action::Rename => "rename cancelled",
            Action::Mkdir => "nothing created",
        }
    }
}

/// A y/N confirmation waiting for its answer.
struct Pending {
    action: Action,
    name: String,
    /// The prompt to go back to.
    prev_prompt: String,
    /// The confirmation prompt (to recognize stale state).
    prompt: String,
    /// Snapshot file for Restore / DropSnapshot.
    file: String,
}

fn picker_id() -> String {
    std::env::var("NOREN_PICKER_ID").unwrap_or_else(|_| "0".into())
}

fn read_pending() -> Option<Pending> {
    let raw = std::fs::read_to_string(pending_file(&picker_id())).ok()?;
    let mut l = raw.split('\n');
    Some(Pending {
        action: Action::from_word(l.next()?)?,
        name: l.next()?.to_string(),
        prev_prompt: l.next()?.to_string(),
        prompt: l.next()?.to_string(),
        file: l.next().unwrap_or("").to_string(),
    })
}

fn write_pending(p: &Pending) {
    let _ = std::fs::write(
        pending_file(&picker_id()),
        format!(
            "{}\n{}\n{}\n{}\n{}",
            p.action.word(),
            p.name,
            p.prev_prompt,
            p.prompt,
            p.file
        ),
    );
}

fn clear_pending() {
    let _ = std::fs::remove_file(pending_file(&picker_id()));
}

fn fzf_prompt() -> String {
    std::env::var("FZF_PROMPT").unwrap_or_default()
}

/// The pending confirmation, if the prompt still shows it.
fn active_pending() -> Option<Pending> {
    let p = read_pending()?;
    if p.prompt == fzf_prompt() {
        Some(p)
    } else {
        clear_pending();
        None
    }
}

/// The session whose snapshot menu the prompt shows, if any.
fn snap_view(prompt: &str) -> Option<&str> {
    prompt.strip_prefix(SNAP_PREFIX).map(str::trim_end)
}

/// The session whose tab menu the prompt shows, if any.
fn tab_view(prompt: &str) -> Option<&str> {
    prompt.strip_prefix(TAB_PREFIX).map(str::trim_end)
}

/// In a per-session menu (snapshots, tabs)?
fn in_menu(prompt: &str) -> bool {
    snap_view(prompt).is_some() || tab_view(prompt).is_some()
}

fn reload(exe: &str, prompt: &str) -> String {
    let cmd = if let Some(name) = snap_view(prompt) {
        format!("{exe} snapshot-rows {}", crate::util::shell_quote(name))
    } else if let Some(name) = tab_view(prompt) {
        format!("{exe} tab-rows {}", crate::util::shell_quote(name))
    } else {
        format!("{exe} list {}", view_source(prompt))
    };
    act("reload", &cmd)
}

/// Preview style chosen with ^/ ("live" | "layout" | "text" | "off");
/// absent = the config's preview_mode.
fn mode_file(id: &str) -> PathBuf {
    runtime_dir().join(format!("noren-pick-{id}.mode"))
}

/// The picker's live/layout override, for `noren preview`.
pub fn preview_mode_override() -> Option<String> {
    let id = std::env::var("NOREN_PICKER_ID").ok()?;
    std::fs::read_to_string(mode_file(&id))
        .ok()
        .filter(|m| matches!(m.as_str(), "tabs" | "live" | "layout" | "text" | "off"))
}

/// Present while the full key overview is shown (F1).
fn keys_file(id: &str) -> PathBuf {
    runtime_dir().join(format!("noren-pick-{id}.keys"))
}

/// Present while the picker lists hidden entries (Alt h).
fn show_hidden_file(id: &str) -> PathBuf {
    runtime_dir().join(format!("noren-pick-{id}.hidden"))
}

/// Is `noren list` running for a picker that currently shows hidden rows?
pub fn showing_hidden() -> bool {
    std::env::var("NOREN_PICKER_ID").is_ok_and(|id| show_hidden_file(&id).exists())
}

/// Where the snapshot menu returns to on esc.
fn view_file() -> PathBuf {
    runtime_dir().join(format!("noren-pick-{}.view", picker_id()))
}

/// Leave confirmation mode: back to the previous prompt (no reload).
fn leave_confirm(p: &Pending) -> String {
    clear_pending();
    [
        "unbind(change)".to_string(),
        "enable-search".to_string(),
        "change-query()".to_string(),
        act("change-prompt", &p.prev_prompt),
    ]
    .join("+")
}

/// Cancel a confirmation: previous prompt + view, status message.
fn restore(exe: &str, p: &Pending, msg: &str) -> String {
    format!(
        "{}+{}+{}",
        leave_confirm(p),
        act("change-ghost", msg),
        reload(exe, &p.prev_prompt)
    )
}

struct Row<'a> {
    source: &'a str,
    name: &'a str,
    path: &'a str,
}

fn parse_row(line: &str) -> Row<'_> {
    let mut parts = line.split('\t');
    Row {
        source: parts.next().unwrap_or(""),
        name: parts.next().unwrap_or(""),
        path: parts.next().unwrap_or(""),
    }
}

/// Run a confirmed (or confirmation-free) kill/delete; fzf actions out.
fn run_kind(exe: &str, kind: Kind, name: &str, state: SessionState, view_prompt: &str) -> String {
    if actions::is_current(name) && matches!(state, SessionState::Live { .. }) {
        // Switches this client away first (discard); on success the picker
        // pane dies with the session. `execute` (not -silent) keeps a
        // failure — e.g. no way to switch away — on screen.
        return act(
            "execute",
            &format!(
                "{exe} {} --yes {} || {{ echo; printf 'press enter to continue '; read _; }}",
                kind.verb(),
                crate::util::shell_quote(name)
            ),
        );
    }
    let msg = match actions::perform(kind, name, state) {
        Ok(m) => format!("✓ {m}"),
        Err(e) => format!("✗ {e}"),
    };
    format!("{}+{}", act("change-ghost", &msg), reload(exe, view_prompt))
}

fn ghost(msg: &str) -> String {
    act("change-ghost", msg)
}

/// `noren pick <event> [row]` — prints fzf actions.
pub fn pick(event: &str, line: &str) {
    let cfg = Config::load();
    let exe = crate::util::shell_quote(&self_exe());
    let row = parse_row(line);
    let out = match event {
        "enter" => match active_pending() {
            Some(p) if p.action == Action::Rename => commit_rename(&exe, &p),
            // ⏎ answers the default: No.
            Some(p) => restore(&exe, &p, p.action.cancelled()),
            None => match row.source {
                "sep" => "ignore".to_string(),
                "snapnew" => snapshot_now(&cfg, &exe, row.name),
                "snapshot" => confirm_snapshot(Action::Restore, &row),
                // Tab menu: connect + land in the tab after fzf exits.
                "tab" => format!(
                    "{}+accept",
                    act("print", &format!("{TAB_MARK}{}\t{}", row.name, row.path))
                ),
                // No match in a menu: nothing to create.
                _ if in_menu(&fzf_prompt()) => "ignore".to_string(),
                // A path-like query means that path (names never start with
                // / ~ ./): go there, or offer to create it — whatever fuzzy
                // match happens to be under the cursor.
                _ if path_query().is_some() => enter_path(&row),
                _ => "accept-or-print-query".to_string(),
            },
        },
        "esc" => match active_pending() {
            Some(p) => restore(&exe, &p, p.action.cancelled()),
            None if in_menu(&fzf_prompt()) => leave_snapshots(&exe),
            None => "abort".to_string(),
        },
        "answer" => match active_pending() {
            None => "unbind(change)".to_string(),
            Some(p) => {
                let q = std::env::var("FZF_QUERY").unwrap_or_default();
                match q.trim() {
                    "" => String::new(),
                    "y" | "Y" => confirmed(&cfg, &exe, &p),
                    _ => restore(&exe, &p, p.action.cancelled()),
                }
            }
        },
        // Timer-driven (auto_reload_secs): quietly re-list the view; never
        // disturb a pending confirmation or an in-progress rename.
        "auto-reload" => match active_pending() {
            Some(_) => String::new(),
            None => reload(&exe, &fzf_prompt()),
        },
        "reload" => match active_pending() {
            Some(p) => restore(&exe, &p, HINT),
            None => format!("{}+{}", ghost(HINT), reload(&exe, &fzf_prompt())),
        },
        "preview-mode" => toggle_preview_mode(&cfg),
        "preview-style" => cycle_preview_style(&cfg),
        "header" => {
            let (cfg, warnings) = Config::load_with_warnings();
            let width = std::env::var("FZF_COLUMNS")
                .ok()
                .and_then(|c| c.parse().ok())
                .map(|c| header_width(&cfg, c));
            let full = keys_file(&picker_id()).exists();
            header(&cfg, &warnings, width, full)
        }
        "help" => {
            // F1: full ↔ compact key overview.
            let f = keys_file(&picker_id());
            if f.exists() {
                let _ = std::fs::remove_file(&f);
            } else {
                let _ = std::fs::write(&f, "");
            }
            act("transform-header", &format!("{exe} pick header"))
        }
        "hide" => hide(&cfg, &exe, line),
        "rename" => start_rename(&row),
        "new-window" => new_window(&cfg, line),
        "paths" => toggle_path_search(),
        "show-hidden" => toggle_show_hidden(&exe),
        "pin" => pin(&cfg, &exe, &row),
        "snapshot" => snapshot_key(&cfg, &exe, &row),
        "tabs" => tabs_key(&exe, &row),
        "delete" if row.source == "snapshot" => confirm_snapshot(Action::DropSnapshot, &row),
        "kill" | "delete" => {
            let kind = if event == "kill" {
                Kind::Kill
            } else {
                Kind::Delete
            };
            confirm_or_run(&cfg, &exe, kind, &row)
        }
        _ => String::new(),
    };
    print!("{out}");
}

fn confirm_or_run(cfg: &Config, exe: &str, kind: Kind, row: &Row) -> String {
    if active_pending().is_some() {
        return String::new();
    }
    if row.source != "zellij" || row.name.is_empty() {
        return ghost(&format!(
            "✗ only sessions can be {}d",
            kind.verb().trim_end_matches('e')
        ));
    }
    let state = actions::session_state(cfg, row.name);
    if !actions::should_confirm(cfg, kind, state) {
        return run_kind(exe, kind, row.name, state, &fzf_prompt());
    }
    let what = match state {
        SessionState::Live { attached: true } if actions::is_current(row.name) => {
            "the session you are in"
        }
        SessionState::Live { attached: true } => "attached session",
        SessionState::Live { attached: false } => "running session",
        _ => "session",
    };
    let prompt = format!("⚠ {} {what} {}? y/N ", kind.verb(), row.name);
    let prev = fzf_prompt();
    write_pending(&Pending {
        action: if kind == Kind::Kill {
            Action::Kill
        } else {
            Action::Delete
        },
        name: row.name.to_string(),
        prev_prompt: if prev.is_empty() {
            DEFAULT_PROMPT.to_string()
        } else {
            prev
        },
        prompt: prompt.clone(),
        file: String::new(),
    });
    [
        "change-query()".to_string(),
        act("change-prompt", &prompt),
        ghost(&format!(
            "y = {} · any other key, ⏎ or esc = cancel",
            kind.verb()
        )),
        "rebind(change)".to_string(),
    ]
    .join("+")
}

/// Alt ⏎: open the selected row in a new terminal window; the picker
/// stays open.
fn new_window(cfg: &Config, line: &str) -> String {
    if active_pending().is_some() {
        return String::new();
    }
    let Some(target) = target_for(line).filter(|_| line.contains('\t')) else {
        return ghost("✗ select a session, folder or config entry");
    };
    if snap_view(&fzf_prompt()).is_some() {
        return ghost("✗ not in the snapshot menu — esc back first");
    }
    match crate::terminal::open(cfg, &self_exe(), &target) {
        Ok(term) => ghost(&format!(
            "↗ opened {} in a new {term} window",
            parse_row(line).name
        )),
        Err(e) => ghost(&format!("✗ {e}")),
    }
}

/// ^e: rename the selected running session — the prompt turns into an
/// input prefilled with the name; ⏎ applies, esc cancels.
fn start_rename(row: &Row) -> String {
    if active_pending().is_some() {
        return String::new();
    }
    if row.source != "zellij" || row.name.is_empty() {
        return ghost("✗ only sessions can be renamed");
    }
    if crate::sources::zellij::session_state(row.name) != Some("live") {
        return ghost("✗ only running sessions can be renamed (zellij can't rename exited ones)");
    }
    let prompt = format!("✎ rename {} → ", row.name);
    write_pending(&Pending {
        action: Action::Rename,
        name: row.name.to_string(),
        prev_prompt: fzf_prompt(),
        prompt: prompt.clone(),
        file: String::new(),
    });
    [
        // Typing edits the name; it mustn't filter the list meanwhile.
        "disable-search".to_string(),
        act("change-prompt", &prompt),
        act("change-query", row.name),
    ]
    .join("+")
}

fn commit_rename(exe: &str, p: &Pending) -> String {
    let new = std::env::var("FZF_QUERY").unwrap_or_default();
    let msg = match crate::session::rename_session(&p.name, &new) {
        Ok(to) => format!("✓ renamed {} → {to}", p.name),
        Err(e) => format!("✗ {e}"),
    };
    restore(exe, p, &msg)
}

/// The query, expanded, when it is path-like.
fn path_query() -> Option<PathBuf> {
    let q = std::env::var("FZF_QUERY").unwrap_or_default();
    let q = q.trim();
    crate::util::is_pathlike(q).then(|| crate::util::expand_path(q))
}

/// ⏎ on a path-like query: the row if it is that path, else the path
/// itself, or — when it doesn't exist — the mkdir offer.
fn enter_path(row: &Row) -> String {
    let Some(p) = path_query() else {
        return "accept-or-print-query".to_string();
    };
    if !p.exists() {
        return confirm_mkdir(&p.to_string_lossy());
    }
    if !row.path.is_empty() && crate::util::expand_path(row.path) == p {
        "accept".to_string()
    } else {
        "print-query".to_string()
    }
}

fn confirm_mkdir(dir: &str) -> String {
    let dir = dir.to_string();
    let shown = crate::util::tilde_path(&dir);
    let prompt = format!("⚠ create {shown} and a session there? y/N ");
    write_pending(&Pending {
        action: Action::Mkdir,
        name: String::new(),
        prev_prompt: fzf_prompt(),
        prompt: prompt.clone(),
        file: dir,
    });
    [
        "change-query()".to_string(),
        act("change-prompt", &prompt),
        ghost("y = mkdir -p + connect · any other key = cancel"),
        "rebind(change)".to_string(),
    ]
    .join("+")
}

/// Alt p: match names only (default) or names + paths. Horizontal
/// scrolling goes off with paths, so a hit deep in a path can't scroll the
/// name out of view.
fn toggle_path_search() -> String {
    let paths_on = std::env::var("FZF_NTH").is_ok_and(|n| n != "1");
    if paths_on {
        "change-nth(1)+toggle-hscroll+change-ghost(searching names)".to_string()
    } else {
        "change-nth(1..)+toggle-hscroll+change-ghost(searching names + paths)".to_string()
    }
}

/// ^h: hide the selected entry, or unhide it (hidden rows are only listed
/// while "show hidden" is on).
fn hide(cfg: &Config, exe: &str, line: &str) -> String {
    let mut fields = line.split('\t');
    let r = crate::sources::Row {
        source: match fields.next().unwrap_or("") {
            "zellij" => "zellij",
            "config" => "config",
            "zoxide" => "zoxide",
            _ => "",
        },
        name: fields.next().unwrap_or("").to_string(),
        path: fields.next().unwrap_or("").to_string(),
        state: "",
        pinned: false,
        icon: String::new(),
        hidden: false,
        tabs: vec![],
    };
    let Some(key) = crate::sources::hidden_key(&r) else {
        return ghost("✗ only sessions, folders and config entries can be hidden");
    };
    let label = if r.source == "zoxide" {
        &r.path
    } else {
        &r.name
    };
    if crate::sources::blacklisted(cfg, &r) {
        // Hidden by the config, not by ^h: only the config can change it.
        return ghost(&format!(
            "✗ {label} is hidden by blacklist in config.toml — remove it there"
        ));
    }
    let hidden_now = crate::state::hidden_toggle(&key);
    let msg = if hidden_now {
        let back = cfg.keys.hidden.trim();
        if back.is_empty() {
            format!("🙈 hid {label}")
        } else {
            format!("🙈 hid {label} · {} shows hidden entries", pretty_key(back))
        }
    } else {
        format!("👁 {label} is visible again")
    };
    format!("{}+{}", ghost(&msg), reload(exe, &fzf_prompt()))
}

/// Alt h: list hidden entries (⊘, dim) alongside the rest, or not.
fn toggle_show_hidden(exe: &str) -> String {
    let f = show_hidden_file(&picker_id());
    let msg = if f.exists() {
        let _ = std::fs::remove_file(&f);
        "hidden entries are hidden again"
    } else {
        let _ = std::fs::write(&f, "");
        "showing hidden entries (⊘) · ^h on one unhides it"
    };
    format!("{}+{}", ghost(msg), reload(exe, &fzf_prompt()))
}

/// The session preview style the config asks for: "tabs" (every tab live,
/// side by side), "live", "layout" or "text".
pub fn configured_style(cfg: &Config) -> &'static str {
    match cfg.preview_mode.as_str() {
        "live" if cfg.live_all_tabs => "tabs",
        "live" => "live",
        "text" => "text",
        _ => "layout",
    }
}

/// Preview styles in cycle order.
const STYLES: &[&str] = &["tabs", "live", "layout", "text"];

fn style_label(style: &str) -> &'static str {
    match style {
        "tabs" => "live · all tabs",
        "live" => "live · active tab",
        "layout" => "layout",
        "text" => "text",
        _ => "off",
    }
}

/// ^/ (combined): cycle all tabs → live → layout → text → off → all tabs.
/// The preview re-renders and labels itself with what it shows.
fn toggle_preview_mode(cfg: &Config) -> String {
    let now = preview_mode_override().unwrap_or_else(|| configured_style(cfg).to_string());
    let next = match STYLES.iter().position(|s| *s == now) {
        Some(i) if i + 1 < STYLES.len() => STYLES[i + 1],
        Some(_) => "off",
        None => STYLES[0],
    };
    let _ = std::fs::write(mode_file(&picker_id()), next);
    let msg = ghost(&format!("preview: {}", style_label(next)));
    match (now.as_str(), next) {
        (_, "off") => format!("{msg}+hide-preview"),
        ("off", _) => format!("{msg}+show-preview+refresh-preview"),
        _ => format!("{msg}+refresh-preview"),
    }
}

/// preview_style key: cycle the style only (never off) — the separate
/// mode's companion to a plain show/hide toggle.
fn cycle_preview_style(cfg: &Config) -> String {
    let now = preview_mode_override()
        .filter(|m| m != "off")
        .unwrap_or_else(|| configured_style(cfg).to_string());
    let i = STYLES.iter().position(|s| *s == now).unwrap_or(0);
    let next = STYLES[(i + 1) % STYLES.len()];
    let _ = std::fs::write(mode_file(&picker_id()), next);
    format!(
        "{}+show-preview+refresh-preview",
        ghost(&format!("preview: {}", style_label(next)))
    )
}

fn pin(cfg: &Config, exe: &str, row: &Row) -> String {
    let msg = match row.source {
        "zellij" if !row.name.is_empty() => {
            let s = crate::state::pin_toggle(row.name);
            if cfg.auto_snapshot_pinned && s == "pinned" {
                let _ = crate::snapshot::take_snapshot(cfg, row.name);
            }
            format!("📌 {s} {}", row.name)
        }
        "zoxide" if !row.path.is_empty() => {
            let s = crate::state::dir_pin_toggle(row.path);
            format!("📌 {s} {}", row.path)
        }
        _ => return ghost("✗ only sessions and folders can be pinned"),
    };
    format!("{}+{}", ghost(&msg), reload(exe, &fzf_prompt()))
}

/// A confirmed action; fzf actions out.
fn confirmed(cfg: &Config, exe: &str, p: &Pending) -> String {
    match p.action {
        Action::Kill | Action::Delete => {
            let kind = if p.action == Action::Kill {
                Kind::Kill
            } else {
                Kind::Delete
            };
            let state = actions::session_state(cfg, &p.name);
            let done = run_kind(exe, kind, &p.name, state, &p.prev_prompt);
            format!("{}+{done}", leave_confirm(p))
        }
        // Restoring may exec zellij (outside) or switch this client
        // (inside): hand it to the picker process after fzf has exited.
        Action::Restore => format!(
            "{}+{}+accept",
            leave_confirm(p),
            act("print", &format!("{RESTORE_MARK}{}\t{}", p.name, p.file))
        ),
        Action::Mkdir => format!(
            "{}+{}+accept-or-print-query",
            leave_confirm(p),
            act("print", &format!("{MKDIR_MARK}{}", p.file))
        ),
        // ⏎ commits a rename (handled in "enter"); typing never confirms.
        Action::Rename => String::new(),
        Action::DropSnapshot => {
            let msg = match std::fs::remove_file(&p.file) {
                Ok(()) => format!("🗑 deleted snapshot {}", file_stem(&p.file)),
                Err(e) => format!("✗ {e}"),
            };
            restore(exe, p, &msg)
        }
    }
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// ^s: on a session, open its snapshot menu; inside the menu, take one.
fn snapshot_key(cfg: &Config, exe: &str, row: &Row) -> String {
    if active_pending().is_some() {
        return String::new();
    }
    let prompt = fzf_prompt();
    if let Some(name) = snap_view(&prompt) {
        return snapshot_now(cfg, exe, name);
    }
    if row.source != "zellij" || row.name.is_empty() {
        return ghost("✗ snapshots are kept per session — select a session");
    }
    let _ = std::fs::write(view_file(), &prompt);
    let menu = format!("{SNAP_PREFIX}{}  ", row.name);
    [
        "change-query()".to_string(),
        act("change-prompt", &menu),
        ghost(SNAP_HINT),
        reload(exe, &menu),
    ]
    .join("+")
}

/// alt-t: open the selected session's tab menu.
fn tabs_key(exe: &str, row: &Row) -> String {
    let prompt = fzf_prompt();
    if active_pending().is_some() || in_menu(&prompt) {
        return String::new();
    }
    if row.source != "zellij" || row.name.is_empty() {
        return ghost("✗ tabs belong to sessions — select a session");
    }
    let _ = std::fs::write(view_file(), &prompt);
    let menu = format!("{TAB_PREFIX}{}  ", row.name);
    [
        "change-query()".to_string(),
        act("change-prompt", &menu),
        ghost(TAB_HINT),
        reload(exe, &menu),
    ]
    .join("+")
}

/// Back from a menu (snapshots, tabs) to the view it was opened from.
fn leave_snapshots(exe: &str) -> String {
    let prev = std::fs::read_to_string(view_file())
        .ok()
        .filter(|p| !p.is_empty() && !in_menu(p))
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let _ = std::fs::remove_file(view_file());
    [
        "change-query()".to_string(),
        act("change-prompt", &prev),
        ghost(HINT),
        reload(exe, &prev),
    ]
    .join("+")
}

fn snapshot_now(cfg: &Config, exe: &str, name: &str) -> String {
    let msg = match crate::snapshot::take_snapshot(cfg, name) {
        Ok(f) => format!(
            "📸 saved {} (keeping {})",
            file_stem(&f.to_string_lossy()),
            cfg.snapshot_keep
        ),
        Err(e) => format!("✗ {e}"),
    };
    format!("{}+{}", ghost(&msg), reload(exe, &fzf_prompt()))
}

/// y/N for restoring from / deleting a snapshot row.
fn confirm_snapshot(action: Action, row: &Row) -> String {
    if action == Action::Restore && actions::is_current(row.name) {
        return ghost(&format!(
            "✗ can't restore {} from inside it — switch to another session first",
            row.name
        ));
    }
    let stem = file_stem(row.path);
    let prompt = match action {
        Action::Restore => format!("⚠ restore {} to {stem}? y/N ", row.name),
        _ => format!("⚠ delete snapshot {stem}? y/N "),
    };
    let hint = match action {
        Action::Restore => {
            "y = restore (the current state is snapshotted first) · any other key = cancel"
        }
        _ => "y = delete this snapshot · any other key = cancel",
    };
    let prev = fzf_prompt();
    write_pending(&Pending {
        action,
        name: row.name.to_string(),
        prev_prompt: prev,
        prompt: prompt.clone(),
        file: row.path.to_string(),
    });
    [
        "change-query()".to_string(),
        act("change-prompt", &prompt),
        ghost(hint),
        "rebind(change)".to_string(),
    ]
    .join("+")
}

/// `noren snapshot-rows <session>` — the snapshot menu's TSV rows: a
/// "take one now" row (next to the prompt), then snapshots newest first.
pub fn snapshot_rows(name: &str) -> String {
    let mut rows = vec![format!(
        "snapnew\t{name}\t\tsnapnew\t＋ take a snapshot of {name} now"
    )];
    for (i, f) in crate::snapshot::list_snapshots(name).iter().enumerate() {
        rows.push(format!(
            "snapshot\t{name}\t{}\tsnapshot\t📸 #{}  {}",
            f.display(),
            i + 1,
            crate::snapshot::describe(f)
        ));
    }
    rows.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_rows_connect_by_name_not_path() {
        let tmp = std::env::temp_dir();
        let line = format!("zellij\tcubic-bee\t{}\tlive\t▢ cubic-bee", tmp.display());
        assert_eq!(target_for(&line).as_deref(), Some("cubic-bee"));
    }

    #[test]
    fn dir_rows_connect_by_existing_path() {
        let tmp = std::env::temp_dir();
        let line = format!("zoxide\ttmp\t{}\tdir\t📁 tmp", tmp.display());
        assert_eq!(target_for(&line), Some(tmp.display().to_string()));
        let gone = "zoxide\tgone\t/definitely/not/here\tdir\tgone";
        assert_eq!(target_for(gone).as_deref(), Some("gone"));
    }

    #[test]
    fn query_without_tabs_is_a_target_and_separator_is_not() {
        assert_eq!(target_for("newproj").as_deref(), Some("newproj"));
        assert_eq!(target_for("sep\t\t\tsep\t╌╌"), None);
        assert_eq!(target_for(""), None);
    }

    #[test]
    fn act_picks_a_free_delimiter() {
        assert_eq!(act("change-ghost", "ok"), "change-ghost(ok)");
        assert_eq!(act("change-ghost", "a (b)"), "change-ghost[a (b)]");
    }

    #[test]
    fn keys_pretty_print() {
        assert_eq!(pretty_key("ctrl-t"), "^t");
        assert_eq!(pretty_key("alt-k"), "M-k");
        assert_eq!(pretty_key("ctrl-alt-d"), "^M-d");
        assert_eq!(pretty_key("f2"), "f2");
    }

    #[test]
    fn compact_header_shows_the_main_keys_and_the_f1_hint() {
        let cfg = Config {
            header_lines: 2,
            ..Config::default()
        };
        let h = header(&cfg, &[], None, false);
        assert_eq!(h.lines().count(), 2, "{h}");
        for k in [
            "^t", "^z", "^a", "^r", "^/", "^p", "^e", "^k", "^d", "⏎", "esc", "f1",
        ] {
            assert!(h.contains(k), "compact header missing {k}: {h}");
        }
        // Secondary keys live in the full overview only.
        for k in ["^s", "^h", "M-h", "M-p"] {
            assert!(!h.contains(k), "compact header shows {k}: {h}");
        }
        // No [[session]] entries: the config view isn't advertised.
        assert!(!h.contains("^f"), "{h}");
        let with_sessions = Config {
            session: vec![crate::config::SessionEntry::default()],
            header_lines: 2,
            ..Config::default()
        };
        assert!(header(&with_sessions, &[], None, false).contains("^f"));
        // connect + close come last on the actions line, F1 after them.
        let line2 = visible(h.lines().nth(1).unwrap());
        assert!(line2.ends_with("⏎ connect  esc close  f1 more"), "{line2}");
    }

    #[test]
    fn hint_moves_up_rather_than_dropping_pin_and_rename() {
        let cfg = Config {
            header_lines: 2,
            ..Config::default()
        };
        // 61 columns = the list pane of a 160-column picker.
        // The hint moves up rather than costing more keys; only the
        // lowest-priority one (pin) gives way to tabs.
        let h = header(&cfg, &[], Some(61), false);
        for k in [
            "^t", "^z", "^a", "^r", "^/", "^w", "^e", "^k", "^d", "⏎", "esc", "f1",
        ] {
            assert!(h.contains(k), "missing {k}: {h}");
        }
        assert!(
            visible(h.lines().next().unwrap()).ends_with("f1 more"),
            "{h}"
        );
        // Roomy: the hint stays in the bottom-right corner.
        let h = header(&cfg, &[], Some(120), false);
        assert!(
            visible(h.lines().nth(1).unwrap()).ends_with("f1 more"),
            "{h}"
        );
    }

    #[test]
    fn full_header_has_every_bind_and_skips_disabled() {
        let mut cfg = Config::default();
        let h = header(&cfg, &[], Some(200), true);
        for k in ["^s", "^h", "M-h", "M-p", "^/", "^e", "f1"] {
            assert!(h.contains(k), "full header missing {k}: {h}");
        }
        assert!(h.contains("less"));
        cfg.keys.snapshot = String::new();
        assert!(!header(&cfg, &[], Some(200), true).contains("snapshot"));
    }

    fn visible(s: &str) -> String {
        let mut out = String::new();
        let mut esc = false;
        for c in s.chars() {
            match (esc, c) {
                (false, '\x1b') => esc = true,
                (true, c) if c.is_ascii_alphabetic() => esc = false,
                (false, c) => out.push(c),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn compact_header_fits_its_line_budget_at_any_width() {
        let mut cfg = Config::default();
        // 160 cols: preview beside (96), list 64 → header width 61.
        assert_eq!(header_width(&cfg, 160), 61);
        // 80 cols: preview would be 48 < 56 → above the list, full width.
        assert_eq!(header_width(&cfg, 80), 77);
        for lines in [1, 2, 3] {
            cfg.header_lines = lines;
            for w in [30, 45, 61, 77, 140] {
                let h = header(&cfg, &[], Some(w), false);
                assert!(h.lines().count() <= lines, "{lines}/{w}: {h}");
                for line in h.lines() {
                    assert!(visible(line).chars().count() <= w, "{lines}/{w}: {line:?}");
                }
                // The most important keys and the F1 hint survive (a single
                // 30-column line only has room for one key besides F1).
                let must: &[&str] = if lines == 1 && w < 40 {
                    &["f1"]
                } else {
                    &["^t", "⏎", "f1"]
                };
                for k in must {
                    assert!(h.contains(k), "{lines}/{w}: missing {k} in {h}");
                }
                // F1 sits at the right edge (bottom line unless moving it
                // up keeps more keys).
                let hint = h
                    .lines()
                    .map(visible)
                    .find(|l| l.ends_with("f1 more"))
                    .unwrap_or_else(|| panic!("{lines}/{w}: no hint in {h}"));
                if w >= 61 && lines >= 2 {
                    assert_eq!(hint.chars().count(), w, "right-aligned: {hint:?}");
                }
            }
        }
        // Full overview wraps without dropping anything.
        let full = header(&Config::default(), &[], Some(30), true);
        for line in full.lines() {
            assert!(visible(line).chars().count() <= 30, "{line:?}");
        }
        assert!(full.contains("M-p") && full.contains("^s"));
    }

    #[test]
    fn duplicate_keys_last_wins_like_fzf() {
        let mut cfg = Config::default();
        cfg.keys.kill = "ctrl-d".into(); // collides with delete (later)
        let b = effective_binds(&cfg);
        assert!(!b.iter().any(|(a, _)| *a == "kill"));
        assert!(b.iter().any(|(a, k)| *a == "delete" && k == "ctrl-d"));
    }

    #[test]
    fn refresh_only_for_live_rows() {
        let st = |t: &str| format!(r#"{{"current":{{"index":0,"text":"{t}"}}}}"#);
        assert!(current_row_is_live(&st("zellij\\tfoo\\t\\tlive\\tfoo")));
        assert!(!current_row_is_live(&st("zellij\\tfoo\\t\\texited\\tfoo")));
        assert!(!current_row_is_live(&st("zoxide\\tx\\t/x\\tdir\\tx")));
        assert!(!current_row_is_live(r#"{"current":null}"#));
        assert!(!current_row_is_live("garbage"));
    }

    #[test]
    fn view_source_from_prompt() {
        assert_eq!(view_source("▢  "), "zellij");
        assert_eq!(view_source("📁  "), "zoxide");
        assert_eq!(view_source("⚙  "), "config");
        assert_eq!(view_source("⚠ kill x? y/N "), "all");
    }
}
