//! CLI surface — must match zjp2 exactly (handover §3.2), plus the v3
//! autostart-integration subcommands `name-for` and `resolve`.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "noren",
    version,
    about = "noren - zellij session picker (sesh parity, Rust)",
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
    /// soft kill (keeps saved layout; asks first if a terminal is attached)
    Kill {
        name: String,
        /// skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// hard delete (removes serialization; asks first if running)
    Delete {
        name: String,
        /// skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// close current session: switch to the previous one, then soft-kill
    Discard { name: Option<String> },
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
    /// save a point-in-time layout snapshot (default: current session)
    Snapshot { name: Option<String> },
    /// list snapshots, newest first (default: current session)
    Snapshots { name: Option<String> },
    /// recreate a session from a snapshot (1 = newest; see `snapshots`)
    Restore {
        name: String,
        #[arg(default_value = "1")]
        index: usize,
        /// skip the confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    /// tabs in session: list, switch, or create
    Window {
        target: Option<String>,
        #[arg(short, long)]
        session: Option<String>,
    },
    /// picker-internal: fzf transform handler for keys and enter/esc
    #[command(hide = true)]
    Pick {
        event: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        line: Vec<String>,
    },
    /// picker-internal: tab menu rows for a session
    #[command(hide = true)]
    TabRows { name: String },
    /// internal: wait for a new client on <session>, then go to tab
    #[command(hide = true)]
    GotoTab {
        session: String,
        position: usize,
        before: usize,
    },
    /// picker-internal: snapshot menu rows for a session
    #[command(hide = true)]
    SnapshotRows { name: String },
    /// fzf preview command (receives the whole TSV line)
    Preview {
        /// live | layout (default: preview_mode from the config)
        #[arg(long)]
        mode: Option<String>,
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
    /// shell completions to stdout: fish | zsh | bash | nushell
    #[command(hide = true)]
    Completions { shell: String },
    /// unknown subcommand falls through to `connect <sub>` (sesh shorthand)
    #[command(external_subcommand)]
    External(Vec<String>),
}
