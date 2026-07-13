//! config source: [[session]] entries from ~/.config/zjp/config.toml.

use crate::config::Config;
use crate::sources::Row;
use crate::util::{expand_path, name_from_path};

pub fn config_sessions(cfg: &Config) -> Vec<Row> {
    cfg.session
        .iter()
        .map(|s| {
            let p = s
                .path
                .as_deref()
                .filter(|p| !p.is_empty())
                .map(|p| expand_path(p).to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = s
                .name
                .clone()
                .unwrap_or_else(|| name_from_path(&p, cfg.dir_length));
            Row {
                source: "config",
                name,
                path: p,
                state: "config",
                pinned: false,
                icon: cfg.icon_for("config", "config", false, false),
            }
        })
        .collect()
}
