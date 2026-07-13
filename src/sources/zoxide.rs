//! zoxide source: `zoxide query -l`, one row per known directory.
//! Dir pins live in zjp3's own state file (paths), never the shared session
//! pin file.

use std::process::Command;

use crate::config::Config;
use crate::sources::Row;
use crate::state::read_dir_pins;
use crate::util::name_from_path;

pub fn zoxide_dirs(cfg: &Config) -> Vec<Row> {
    let Ok(out) = Command::new("zoxide").args(["query", "-l"]).output() else {
        return vec![];
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let pins = read_dir_pins();
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|p| {
            let pinned = pins.iter().any(|d| d == p);
            Row {
                source: "zoxide",
                name: name_from_path(p, cfg.dir_length),
                path: p.to_string(),
                state: "dir",
                pinned,
                icon: cfg.icon_for("zoxide", "dir", pinned),
            }
        })
        .collect()
}
