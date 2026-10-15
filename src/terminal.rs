//! Open a target in a NEW terminal window (picker alt-enter): the window
//! runs `noren connect <target>`, so creating, resurrecting and layouts
//! work exactly as from a shell.
//!
//! Terminal: `terminal` from the config — a name/path ("foot") or a
//! template with `{}` for the command ("footclient -a noren {}") — else
//! $TERMINAL, else the first known emulator on PATH.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::util::shell_quote;

/// Tried in order when neither the config nor $TERMINAL names one.
const KNOWN: &[&str] = &[
    "foot",
    "kitty",
    "ghostty",
    "alacritty",
    "wezterm",
    "konsole",
    "gnome-terminal",
    "xterm",
];

fn on_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// Arguments that make `term` run a command: (prefix after the binary).
fn exec_args(term: &str) -> &'static [&'static str] {
    let base = Path::new(term)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match base.as_str() {
        "foot" | "footclient" | "kitty" => &[],
        "wezterm" => &["start", "--"],
        "gnome-terminal" | "kgx" => &["--"],
        _ => &["-e"],
    }
}

/// How to launch: a template (run through sh) or a terminal binary.
enum Launcher {
    Template(String),
    Binary(PathBuf),
}

fn launcher(cfg: &Config) -> Option<Launcher> {
    let configured = cfg.terminal.trim();
    if configured.contains("{}") {
        return Some(Launcher::Template(configured.to_string()));
    }
    if !configured.is_empty() {
        return on_path(configured).map(Launcher::Binary);
    }
    std::env::var("TERMINAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| on_path(t.trim()))
        .or_else(|| KNOWN.iter().find_map(|t| on_path(t)))
        .map(Launcher::Binary)
}

/// Start `exe connect <target>` in a new terminal window, detached from the
/// picker. Returns the terminal's name for the status message.
pub fn open(cfg: &Config, exe: &str, target: &str) -> anyhow::Result<String> {
    let Some(launcher) = launcher(cfg) else {
        anyhow::bail!("no terminal found — set `terminal` in config.toml or $TERMINAL");
    };
    let (mut cmd, name) = match launcher {
        Launcher::Template(t) => {
            let inner = format!("{} connect {}", shell_quote(exe), shell_quote(target));
            let mut c = Command::new("sh");
            c.arg("-c").arg(t.replace("{}", &inner));
            let name = t
                .split_whitespace()
                .next()
                .unwrap_or("terminal")
                .to_string();
            (c, name)
        }
        Launcher::Binary(bin) => {
            let mut c = Command::new(&bin);
            c.args(exec_args(&bin.to_string_lossy()))
                .arg(exe)
                .args(["connect", target]);
            let name = bin
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (c, name)
        }
    };
    // The new window is a fresh client OUTSIDE zellij: it must not inherit
    // this pane's session (connect would switch in place instead of
    // attaching) nor the picker's state.
    for var in [
        "ZELLIJ",
        "ZELLIJ_SESSION_NAME",
        "ZELLIJ_PANE_ID",
        "NOREN_PICKER_ID",
        "NOREN_QUIET",
        "FZF_SOCK",
    ] {
        cmd.env_remove(var);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Own process group: survives the picker closing.
        .process_group(0)
        .spawn()
        .map_err(|e| anyhow::anyhow!("couldn't start {name}: {e}"))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::exec_args;

    #[test]
    fn exec_args_per_terminal() {
        assert!(exec_args("/usr/bin/foot").is_empty());
        assert!(exec_args("kitty").is_empty());
        assert_eq!(exec_args("wezterm"), ["start", "--"]);
        assert_eq!(exec_args("gnome-terminal"), ["--"]);
        assert_eq!(exec_args("ghostty"), ["-e"]);
        assert_eq!(exec_args("some-unknown-term"), ["-e"]);
    }
}
