//! `noren completions <shell>` — clap-generated completions for
//! fish/zsh/bash/nushell, post-processed so session-taking arguments
//! complete **session names** (live/exited + config entries) instead of
//! falling back to file paths. Candidates are queried live from
//! `noren list` at completion time (~20 ms).

use clap::CommandFactory;
use clap_complete::{Generator, generate};

fn generate_to_string(generator: impl Generator) -> String {
    let mut cmd = crate::cli::Cli::command();
    let mut buf = Vec::new();
    generate(generator, &mut cmd, "noren", &mut buf);
    String::from_utf8(buf).unwrap_or_default()
}

pub fn completions_cmd(shell: &str) {
    match shell {
        "bash" => print!("{}", generate_to_string(clap_complete::Shell::Bash)),
        "zsh" => print!("{}", zsh_with_sessions()),
        "fish" => {
            print!("{}", generate_to_string(clap_complete::Shell::Fish));
            print!("{FISH_DYNAMIC}");
        }
        "nushell" | "nu" => print!("{}", nushell_with_sessions()),
        other => {
            eprintln!("noren completions: unknown shell \"{other}\" (fish|zsh|bash|nushell)");
            std::process::exit(1);
        }
    }
}

// Sessions only by default (no dir paths): completing a zoxide dir by its
// short display name would create a wrong session in $PWD, and flooding the
// candidates with paths buries the sessions. Paths can still be typed.
const FISH_DYNAMIC: &str = r#"
# ---- noren dynamic completions (appended after clap's static set) ----
function __noren_sessions
    command noren list zellij 2>/dev/null | string split -f2 \t | string match -rv '^$'
    command noren list config 2>/dev/null | string split -f2 \t | string match -rv '^$'
end
complete -c noren -n __fish_use_subcommand -f -a '(__noren_sessions)'
complete -c noren -n '__fish_seen_subcommand_from connect kill delete pin discard snapshot snapshots restore resolve' -f -a '(__noren_sessions)'
"#;

/// clap's zsh output completes positionals with `_default` (files). Swap the
/// session-taking ones for a live session lookup; paths (mkdir/root/clone),
/// rename and the restore index keep file/default behavior.
fn zsh_with_sessions() -> String {
    let out = generate_to_string(clap_complete::Shell::Zsh);
    let func = r#"
_noren_sessions() {
    local -a sessions
    sessions=(${(f)"$(command noren list zellij 2>/dev/null | cut -f2)"})
    sessions+=(${(f)"$(command noren list config 2>/dev/null | cut -f2)"})
    _describe -t sessions 'zellij session' sessions
}
"#;
    let out = out
        .replace("':target:_default'", "':target:_noren_sessions'")
        .replace("':name:_default'", "':name:_noren_sessions'")
        .replace("'::name:_default'", "'::name:_noren_sessions'");
    // Inject the helper right after the `#compdef` line.
    match out.split_once('\n') {
        Some((first, rest)) => format!("{first}\n{func}\n{rest}"),
        None => out,
    }
}

/// Same for nushell: attach a completer to the session-taking `string`
/// params. The def lives inside the generated module, before the externs
/// (the standard nushell completion-file pattern).
fn nushell_with_sessions() -> String {
    let out = generate_to_string(clap_complete_nushell::Nushell);
    let func = r#"
  def "nu-complete noren sessions" [] {
    ^noren list zellij err> /dev/null
    | lines
    | append (^noren list config err> /dev/null | lines)
    | each {|l| $l | split row "\t" | get 1 }
  }
"#;
    out.replacen(
        "module completions {\n",
        &format!("module completions {{\n{func}"),
        1,
    )
    .replace(
        "    target: string\n",
        "    target: string@\"nu-complete noren sessions\"\n",
    )
    .replace(
        "    name: string\n",
        "    name: string@\"nu-complete noren sessions\"\n",
    )
    .replace(
        "    name?: string\n",
        "    name?: string@\"nu-complete noren sessions\"\n",
    )
}
