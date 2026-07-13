//! `zjp3 completions <shell>` — static clap-generated completions for
//! fish/zsh/bash/nushell. Fish additionally gets dynamic candidates that
//! query `zjp3 list` at completion time (session names for session-taking
//! subcommands, names + dir paths for connect and the bare shorthand).

use clap::CommandFactory;
use clap_complete::generate;

pub fn completions_cmd(shell: &str) {
    let mut cmd = crate::cli::Cli::command();
    let mut out = std::io::stdout();
    match shell {
        "bash" => generate(clap_complete::Shell::Bash, &mut cmd, "zjp3", &mut out),
        "zsh" => generate(clap_complete::Shell::Zsh, &mut cmd, "zjp3", &mut out),
        "fish" => {
            generate(clap_complete::Shell::Fish, &mut cmd, "zjp3", &mut out);
            print!("{FISH_DYNAMIC}");
        }
        "nushell" | "nu" => generate(clap_complete_nushell::Nushell, &mut cmd, "zjp3", &mut out),
        other => {
            eprintln!("zjp3 completions: unknown shell \"{other}\" (fish|zsh|bash|nushell)");
            std::process::exit(1);
        }
    }
}

// Dirs complete by path, not display name: connecting to a bare short name
// that isn't a session would create a fresh session in $PWD instead.
const FISH_DYNAMIC: &str = r#"
# ---- zjp3 dynamic completions (appended after clap's static set) ----
function __zjp3_sessions
    command zjp3 list zellij 2>/dev/null | string split -f2 \t | string match -rv '^$'
end
function __zjp3_targets
    __zjp3_sessions
    command zjp3 list config 2>/dev/null | string split -f2 \t | string match -rv '^$'
    command zjp3 list zoxide 2>/dev/null | string split -f3 \t | string match -rv '^$'
end
complete -c zjp3 -n __fish_use_subcommand -f -a '(__zjp3_targets)'
complete -c zjp3 -n '__fish_seen_subcommand_from connect' -f -a '(__zjp3_targets)'
complete -c zjp3 -n '__fish_seen_subcommand_from kill delete pin discard snapshot snapshots restore' -f -a '(__zjp3_sessions)'
"#;
