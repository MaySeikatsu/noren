//! Tab menu (picker alt-t): a session's tabs as rows; ⏎ connects to the
//! session AND lands in that tab.
//!
//! Landing (verified on zellij 0.45.1): the zellij-switch plugin can't
//! target a tab, and `zellij action go-to-tab` from the CLI moves the
//! NEWEST client of the session — which is exactly the client that just
//! switched in or attached. So: connect first, wait until the client has
//! arrived, then go-to-tab. Outside zellij the connect execs `zellij
//! attach`, so a detached `noren goto-tab` helper does the waiting.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::connect::{inside_zellij, session_connect};
use crate::sources::zellij::{
    action_output_with_timeout, attached_clients, cached_layout_file, session_state, wait_until,
};
use crate::util::tilde_path;

/// One tab of a session, 1-based like `go-to-tab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabInfo {
    pub position: usize,
    pub name: String,
    /// An attached client looks at it (live) / focused when saved (exited).
    pub active: bool,
    /// Focused pane title · cwd (live sessions only).
    pub detail: String,
}

/// Tabs of a session: live ones ask zellij, exited ones read the saved
/// layout (never contacted).
pub fn tabs_of(name: &str) -> Vec<TabInfo> {
    if session_state(name) == Some("live")
        && let Some(tabs) = crate::live_preview::tab_summaries(name)
    {
        return tabs;
    }
    let src = std::fs::read_to_string(cached_layout_file(name)).unwrap_or_default();
    let focused = crate::layout_preview::session_focus(name).map(|f| f.tab);
    crate::layout_preview::tab_names(&src)
        .into_iter()
        .enumerate()
        .map(|(i, n)| TabInfo {
            position: i + 1,
            active: focused.as_deref() == Some(n.as_str()),
            name: n,
            detail: String::new(),
        })
        .collect()
}

/// `noren tab-rows <session>` — TSV for the picker's tab menu:
/// `tab \t <session> \t <position> \t tab \t <display>`.
pub fn tab_rows(name: &str) -> String {
    let live = session_state(name) == Some("live");
    let tabs = tabs_of(name);
    if tabs.is_empty() {
        return format!("tabnone\t{name}\t\ttabnone\t\x1b[2m(no tabs known for {name})\x1b[0m");
    }
    let width = tabs
        .iter()
        .map(|t| t.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(24);
    tabs.iter()
        .map(|t| {
            let mark = if t.active { "●" } else { " " };
            let label = t.name.replace('\t', " ");
            let fill = width.saturating_sub(label.chars().count());
            let detail = if live {
                t.detail.clone()
            } else {
                "saved layout".to_string()
            };
            format!(
                "tab\t{name}\t{}\ttab\t▤ {} {mark} {label}{:fill$}  \x1b[2m{detail}\x1b[0m",
                t.position, t.position, ""
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `zellij action go-to-tab <position>` on a RUNNING session (checked —
/// an unknown name would make zellij act on some other session).
fn go_to_tab(session: &str, position: usize) -> anyhow::Result<()> {
    if session_state(session) != Some("live") {
        anyhow::bail!("\"{session}\" isn't running");
    }
    let out = action_output_with_timeout(session, &["go-to-tab", &position.to_string()], 1500);
    if out.is_none() {
        anyhow::bail!("zellij action go-to-tab {position} failed");
    }
    Ok(())
}

/// Connect to `session` and land in tab `position`.
pub fn connect_tab(cfg: &Config, session: &str, position: usize) -> anyhow::Result<()> {
    let current = std::env::var("ZELLIJ_SESSION_NAME").unwrap_or_default();
    let before = if session_state(session) == Some("live") {
        attached_clients(session)
    } else {
        0
    };
    if inside_zellij() {
        if current == session {
            return go_to_tab(session, position);
        }
        session_connect(cfg, session)?;
        // The client lands a few ms after the switch returns.
        wait_until(2000, 25, || {
            session_state(session) == Some("live") && attached_clients(session) > before
        });
        return go_to_tab(session, position);
    }
    // Outside: connect execs `zellij attach` — leave the tab switch to a
    // detached helper that waits for this client to show up.
    let exe = std::env::current_exe()?;
    let _ = Command::new(exe)
        .args([
            "goto-tab",
            session,
            &position.to_string(),
            &before.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    session_connect(cfg, session)
}

/// `noren goto-tab <session> <position> <clients-before>` (hidden): wait
/// until a new client is attached, then move it to the tab.
pub fn goto_helper(session: &str, position: usize, before: usize) {
    let arrived = wait_until(8000, 50, || {
        session_state(session) == Some("live") && attached_clients(session) > before
    });
    if arrived {
        // Let the client finish attaching before redirecting it.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = go_to_tab(session, position);
    }
}

/// Detail line for a live tab: focused pane title · cwd.
pub fn detail(title: &str, cwd: Option<&str>) -> String {
    match cwd.map(tilde_path).filter(|c| !c.is_empty()) {
        Some(c) if !title.is_empty() => format!("{title} · {c}"),
        Some(c) => c,
        None => title.to_string(),
    }
}
