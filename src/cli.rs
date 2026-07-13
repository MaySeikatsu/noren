//! CLI surface — must match zjp2 exactly (handover §3.2), plus the v3
//! autostart-integration subcommands `name-for` and `resolve`.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "zjp3",
    about = "zjp3 - zellij session picker (sesh parity, Rust)",
    disable_help_subcommand = false
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// list rows (default: all)
    List {
        /// zellij | zoxide | config | all | blacklist
        #[arg(default_value = "all")]
        source: String,
    },
    /// attach or create
    Connect { target: String },
    /// switch to previous session
    Last,
    /// connect to git top-level
    Root { path: Option<String> },
    /// soft kill (keeps saved layout)
    Kill { name: String },
    /// hard delete (removes serialization)
    Delete { name: String },
    /// mkdir -p + connect
    Mkdir { path: String },
    /// git clone + connect
    Clone { url: String, dest: Option<String> },
    /// toggle pin (default: current session)
    Pin { name: Option<String> },
    /// picker-internal: pin toggle on a whole TSV row (sessions by name,
    /// dirs by path)
    #[command(hide = true)]
    PinRow {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        line: Vec<String>,
    },
    /// rename current session (also updates pin)
    Rename { new: String },
    /// tabs in session: list, switch, or create
    Window {
        target: Option<String>,
        #[arg(short, long)]
        session: Option<String>,
    },
    /// fzf preview command (receives the whole TSV line)
    Preview {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        line: Vec<String>,
    },
    /// interactive picker (also the default with no args)
    Picker,
    /// sanitized session name for a path (autostart integration)
    NameFor { path: String },
    /// resolve a target to name/path/layout/startup (autostart integration)
    Resolve {
        target: String,
        /// env | json
        #[arg(long, default_value = "env")]
        format: String,
    },
    /// unknown subcommand falls through to `connect <sub>` (sesh shorthand)
    #[command(external_subcommand)]
    External(Vec<String>),
}
