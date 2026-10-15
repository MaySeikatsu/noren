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
/// Preview labels; the live/layout toggle reads the current one back from
/// $FZF_PREVIEW_LABEL.
const LABEL_LIVE: &str = " ● live ";
const LABEL_LAYOUT: &str = " ▦ layout ";

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

/// "ctrl-t" -> "^t", "alt-x" -> "M-x", "ctrl-alt-d" -> "^M-d".
fn pretty_key(key: &str) -> String {
    let mut k = key.trim().to_string();
    for (from, to) in [("ctrl-", "^"), ("alt-", "M-")] {
        k = k.replace(from, to);
    }
    k
}

/// Two header lines: views + list controls, then row actions. Keys bold,
/// labels dim; config warnings (if any) on top in yellow.
fn header(cfg: &Config, warnings: &[String]) -> String {
    let binds = effective_binds(cfg);
    let item = |action: &str, label: &str| {
        binds
            .iter()
            .find(|(a, _)| *a == action)
            .map(|(_, k)| format!("\x1b[1m{}\x1b[0m \x1b[2m{label}\x1b[0m", pretty_key(k)))
    };
    let line = |items: Vec<Option<String>>, tail: &str| {
        let mut parts: Vec<String> = items.into_iter().flatten().collect();
        if !tail.is_empty() {
            parts.push(tail.to_string());
        }
        parts.join("  ")
    };
    let views = line(
        vec![
            item("sessions", "sessions"),
            item("dirs", "dirs"),
            item("config", "config"),
            item("all", "all"),
            item("reload", "reload"),
            item("preview", "preview"),
            item("live", "live/layout"),
        ],
        "",
    );
    let acts = line(
        vec![
            item("pin", "pin"),
            item("snapshot", "snapshot"),
            item("kill", "kill"),
            item("delete", "delete"),
        ],
        "\x1b[1m⏎\x1b[0m \x1b[2mconnect\x1b[0m  \x1b[1mesc\x1b[0m \x1b[2mclose\x1b[0m",
    );
    let mut out: Vec<String> = warnings
        .iter()
        .map(|w| format!("\x1b[33m⚠ {w}\x1b[0m"))
        .collect();
    out.push(views);
    out.push(acts);
    out.join("\n")
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
            "preview" => "toggle-preview".to_string(),
            "live" => format!("transform({exe} pick preview-mode)"),
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
    binds.push("start:unbind(change)".to_string());
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
        &header(&cfg, &warnings),
        // Binds are written for POSIX sh, whatever the user's $SHELL is.
        "--with-shell",
        "sh -c",
        "--preview",
        &format!("{exe} preview {{}}"),
        "--preview-label",
        if cfg.preview_mode == "live" {
            LABEL_LIVE
        } else {
            LABEL_LAYOUT
        },
        "--preview-window",
        &cfg.preview_window(),
    ]);
    for b in fzf_binds(&cfg, &exe) {
        cmd.arg("--bind").arg(b);
    }
    // Live previews refresh on a timer through fzf's HTTP API.
    // Unix socket paths max out at ~107 bytes; skip refreshing rather than
    // fail to start fzf in an exotic runtime dir.
    let sock =
        Some(fzf_socket(&id)).filter(|p| cfg.live_refresh_ms > 0 && p.as_os_str().len() < 100);
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
    let refresher = sock.clone().map(|sock| {
        let stop = stop.clone();
        let every = std::time::Duration::from_millis(cfg.live_refresh_ms.max(200));
        std::thread::spawn(move || refresh_live_preview(&sock, every, &stop))
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

    // Non-zero = esc / ctrl-c abort; just close the picker.
    if !out.status.success() {
        return Ok(());
    }
    let line = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let Some(target) = target_for(&line) else {
        return Ok(());
    };
    let result = session_connect(&cfg, &target);
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

/// While the picker runs: every `every`, re-render the preview if the
/// cursor sits on a live session (other previews don't change on their
/// own, and re-running e.g. eza would only flicker).
fn refresh_live_preview(
    sock: &std::path::Path,
    every: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    let tick = std::time::Duration::from_millis(50);
    let mut waited = std::time::Duration::ZERO;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(tick);
        waited += tick;
        if waited < every {
            continue;
        }
        waited = std::time::Duration::ZERO;
        if fzf_http(sock, "GET", "").is_some_and(|st| current_row_is_live(&st)) {
            let _ = fzf_http(sock, "POST", "refresh-preview");
        }
    }
}

// ---- `noren pick <event>`: fzf transform handlers -------------------------

/// An fzf action with an argument, using a delimiter pair the argument
/// doesn't contain (names/messages may contain parentheses).
fn act(name: &str, arg: &str) -> String {
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

/// A y/N confirmation waiting for its answer.
struct Pending {
    kind: Kind,
    name: String,
    /// The prompt to go back to.
    prev_prompt: String,
    /// The confirmation prompt (to recognize stale state).
    prompt: String,
}

fn picker_id() -> String {
    std::env::var("NOREN_PICKER_ID").unwrap_or_else(|_| "0".into())
}

fn read_pending() -> Option<Pending> {
    let raw = std::fs::read_to_string(pending_file(&picker_id())).ok()?;
    let mut l = raw.split('\n');
    let kind = match l.next()? {
        "kill" => Kind::Kill,
        "delete" => Kind::Delete,
        _ => return None,
    };
    Some(Pending {
        kind,
        name: l.next()?.to_string(),
        prev_prompt: l.next()?.to_string(),
        prompt: l.next()?.to_string(),
    })
}

fn write_pending(p: &Pending) {
    let _ = std::fs::write(
        pending_file(&picker_id()),
        format!(
            "{}\n{}\n{}\n{}",
            p.kind.verb(),
            p.name,
            p.prev_prompt,
            p.prompt
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

fn reload(exe: &str, prompt: &str) -> String {
    act("reload", &format!("{exe} list {}", view_source(prompt)))
}

/// Leave confirmation mode: back to the previous prompt (no reload).
fn leave_confirm(p: &Pending) -> String {
    clear_pending();
    [
        "unbind(change)".to_string(),
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
    format!("{}+{}", reload(exe, view_prompt), act("change-ghost", &msg))
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
            // ⏎ answers the default: No.
            Some(p) => restore(&exe, &p, &format!("{} cancelled", p.kind.verb())),
            None if row.source == "sep" => "ignore".to_string(),
            None => "accept-or-print-query".to_string(),
        },
        "esc" => match active_pending() {
            Some(p) => restore(&exe, &p, &format!("{} cancelled", p.kind.verb())),
            None => "abort".to_string(),
        },
        "answer" => match active_pending() {
            None => "unbind(change)".to_string(),
            Some(p) => {
                let q = std::env::var("FZF_QUERY").unwrap_or_default();
                match q.trim() {
                    "" => String::new(),
                    "y" | "Y" => {
                        let state = actions::session_state(&cfg, &p.name);
                        let done = run_kind(&exe, p.kind, &p.name, state, &p.prev_prompt);
                        format!("{}+{done}", leave_confirm(&p))
                    }
                    _ => restore(&exe, &p, &format!("{} cancelled", p.kind.verb())),
                }
            }
        },
        "reload" => match active_pending() {
            Some(p) => restore(&exe, &p, HINT),
            None => format!("{}+{}", reload(&exe, &fzf_prompt()), ghost(HINT)),
        },
        "preview-mode" => toggle_preview_mode(&exe),
        "pin" => pin(&cfg, &exe, &row),
        "snapshot" => snapshot(&cfg, &row),
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
        kind,
        name: row.name.to_string(),
        prev_prompt: if prev.is_empty() {
            DEFAULT_PROMPT.to_string()
        } else {
            prev
        },
        prompt: prompt.clone(),
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

/// Live ↔ layout for session previews, tracked in the preview label.
fn toggle_preview_mode(exe: &str) -> String {
    let now_live = std::env::var("FZF_PREVIEW_LABEL").is_ok_and(|l| l == LABEL_LIVE);
    let (mode, label) = if now_live {
        ("layout", LABEL_LAYOUT)
    } else {
        ("live", LABEL_LIVE)
    };
    format!(
        "{}+{}",
        act(
            "change-preview",
            &format!("{exe} preview --mode {mode} {{}}")
        ),
        act("change-preview-label", label)
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
    format!("{}+{}", reload(exe, &fzf_prompt()), ghost(&msg))
}

fn snapshot(cfg: &Config, row: &Row) -> String {
    if row.source != "zellij" || row.name.is_empty() {
        return ghost("✗ snapshots are taken of sessions");
    }
    match crate::snapshot::take_snapshot(cfg, row.name) {
        Ok(_) => ghost(&format!(
            "📸 snapshot of {} saved (keeping {})",
            row.name, cfg.snapshot_keep
        )),
        Err(e) => ghost(&format!("✗ {e}")),
    }
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
    fn header_lists_every_active_bind_and_skips_disabled() {
        let mut cfg = Config::default();
        let h = header(&cfg, &[]);
        for k in [
            "^t", "^z", "^f", "^a", "^r", "^/", "^l", "^p", "^s", "^k", "^d",
        ] {
            assert!(h.contains(k), "header missing {k}: {h}");
        }
        cfg.keys.snapshot = String::new();
        assert!(!header(&cfg, &[]).contains("snapshot"));
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
