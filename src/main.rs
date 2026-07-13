mod cli;
mod completions;
mod config;
mod connect;
mod layout_preview;
mod picker;
mod preview;
mod session;
mod snapshot;
mod sources;
mod state;
mod util;
mod window;

use std::io::{IsTerminal, Write};
use std::process::Command;

use clap::Parser;

use cli::{Cli, Cmd};
use config::Config;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        None | Some(Cmd::Picker) => picker::picker(&Config::load()),
        Some(Cmd::List { source }) => {
            sources::list_cmd(&Config::load(), &source);
            Ok(())
        }
        Some(Cmd::Connect { target }) => connect::session_connect(&Config::load(), &target),
        Some(Cmd::Last) => {
            let prev = state::read_last();
            if prev.is_empty() {
                eprintln!("noren last: no previous session recorded yet");
                std::process::exit(1);
            }
            connect::session_connect(&Config::load(), &prev)
        }
        Some(Cmd::Root { path }) => {
            let p = path
                .as_deref()
                .map(util::expand_path)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| util::home_dir()));
            let top = Command::new("git")
                .arg("-C")
                .arg(&p)
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if top.is_empty() {
                eprintln!("noren root: {} is not inside a git repository", p.display());
                std::process::exit(1);
            }
            connect::session_connect(&Config::load(), &top)
        }
        Some(Cmd::Kill { name }) => {
            let status = Command::new("zellij")
                .args(["kill-session", &name])
                .status()?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Some(Cmd::Delete { name }) => {
            // Confirm on a tty; skip when stdin isn't one (fzf execute-silent).
            if std::io::stdin().is_terminal() {
                print!("Hard-delete zellij session \"{name}\"? [y/N] ");
                std::io::stdout().flush().ok();
                let mut ans = String::new();
                std::io::stdin().read_line(&mut ans).ok();
                if !matches!(ans.trim(), "y" | "Y" | "yes" | "Yes") {
                    println!("aborted");
                    return Ok(());
                }
            }
            let status = Command::new("zellij")
                .args(["delete-session", &name])
                .status()?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Some(Cmd::Mkdir { path }) => {
            let p = util::expand_path(&path);
            if !p.exists() {
                std::fs::create_dir_all(&p)?;
            }
            connect::session_connect(&Config::load(), &p.to_string_lossy())
        }
        Some(Cmd::Clone { url, dest }) => {
            let dest = dest.unwrap_or_else(|| {
                let base = url.rsplit('/').next().unwrap_or(&url);
                base.strip_suffix(".git").unwrap_or(base).to_string()
            });
            let p = util::expand_path(&dest);
            if !p.exists() {
                let status = Command::new("git")
                    .arg("clone")
                    .arg(&url)
                    .arg(&p)
                    .status()?;
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
            connect::session_connect(&Config::load(), &p.to_string_lossy())
        }
        Some(Cmd::Discard { name }) => connect::discard(&Config::load(), name.as_deref()),
        Some(Cmd::Pin { name }) => {
            session::pin_cmd(&Config::load(), name.as_deref());
            Ok(())
        }
        Some(Cmd::PinRow { line }) => {
            session::pin_row(&Config::load(), &line.join(" "));
            Ok(())
        }
        Some(Cmd::Snapshot { name }) => {
            snapshot::snapshot_cmd(&Config::load(), name.as_deref());
            Ok(())
        }
        Some(Cmd::Snapshots { name }) => {
            snapshot::snapshots_cmd(name.as_deref());
            Ok(())
        }
        Some(Cmd::Restore { name, index, force }) => {
            snapshot::restore_cmd(&Config::load(), &name, index, force)
        }
        Some(Cmd::Completions { shell }) => {
            completions::completions_cmd(&shell);
            Ok(())
        }
        Some(Cmd::Rename { new }) => {
            session::rename_current(&new);
            Ok(())
        }
        Some(Cmd::Window { target, session }) => {
            window::window(target.as_deref(), session.as_deref());
            Ok(())
        }
        Some(Cmd::Preview { line }) => {
            preview::preview(&Config::load(), &line.join(" "));
            Ok(())
        }
        Some(Cmd::NameFor { path }) => {
            println!("{}", name_for(&Config::load(), &path));
            Ok(())
        }
        Some(Cmd::Resolve { target, format }) => {
            resolve_cmd(&Config::load(), &target, &format);
            Ok(())
        }
        Some(Cmd::External(args)) => {
            // Unknown subcommand -> connect target (sesh-ish shorthand).
            let target = args.first().cloned().unwrap_or_default();
            connect::session_connect(&Config::load(), &target)
        }
    }
}

/// Session name for a path, as zellij-autostart would pick it: the resolver's
/// name, with autostart's `$HOME -> "home"` special case preserved.
fn name_for(cfg: &Config, path: &str) -> String {
    if util::expand_path(path) == util::home_dir() {
        return "home".to_string();
    }
    connect::resolve(cfg, path).name
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `noren resolve <target> --format=env|json` for shell consumers
/// (zellij-autostart evals the env form).
fn resolve_cmd(cfg: &Config, target: &str, format: &str) {
    let r = connect::resolve(cfg, target);
    match format {
        "json" => println!("{}", serde_json::to_string(&r).expect("serializable")),
        _ => {
            println!("NAME={}", shell_quote(&r.name));
            println!("SESSION_PATH={}", shell_quote(&r.path));
            println!("LAYOUT={}", shell_quote(&r.layout));
            println!("STARTUP={}", shell_quote(&r.startup_command));
        }
    }
}
