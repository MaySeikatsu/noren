# noren (暖簾)

**A sesh-style session manager for [zellij](https://zellij.dev). Brush through the curtain into any project.**

A *noren* is the fabric curtain hanging in the doorway of a Japanese shop — you don't open it, you brush through it without breaking stride. That's the feel this tool is after: one keypress, a fuzzy match, and you're standing in another project.

noren merges your **running zellij sessions**, **[zoxide](https://github.com/ajeetdsouza/zoxide) directories**, and **configured project entries** into a single [fzf](https://github.com/junegunn/fzf) picker, and connects to whatever you choose — attaching, creating, resurrecting, or switching in place as appropriate. Feature parity with [sesh](https://github.com/joshmedeski/sesh) (tmux), plus a few things sesh doesn't have.

```
⚡  nixos▏
   📌 ▣ nixos          ← pinned, open in a terminal right now   ┌─ preview ────────────┐
   📌 ▢ shellseikatsu  ← pinned, running in the background      │ ┏━━━━━━━┓┌─────────┐ │
   ▣ hestia_tauri                                               │ ┃ hx  * ┃│ cargo   │ │
   ⊗ old-experiment    ← dead, resurrectable                    │ ┗━━━━━━━┛└─────────┘ │
   ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌                                           │ (+ 1 floating)       │
   ⚙ dotfiles          ← from config.toml                       └──────────────────────┘
   📁 rust/tutorials   ← from zoxide
```

## Features

- **One picker, three sources** — live/exited sessions, zoxide dirs, `[[session]]` config entries; deduplicated (sessions beat config beat dirs), sorted so the most relevant row sits next to the prompt, with a dim hairline separating sessions from folders.
- **Session-aware everywhere** — inside zellij it switches your client in place (via the [zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch) plugin — never nested attaches); outside it exec's `zellij attach --create`.
- **Visual previews without resurrection** — the pane layout drawn as box art (focused pane highlighted, running programs labeled), rendered *only* from zellij's serialized layout cache. Hovering a dead session can never wake it.
- **Attached indicator** — ▣ marks sessions currently shown in a terminal, ▢ ones running in the background.
- **Pinning** — pin sessions (shared, name-based) and folders (separate, path-based); pinned rows float to the prompt.
- **Snapshots** — point-in-time backups of a session's layout + running commands; restore any of the last N. Optionally automatic for pinned sessions.
- **Project rules** — `[[session]]` entries and `[[wildcard]]` globs assign layouts, startup commands, and preview commands per project; `noren resolve` exposes the same resolver to shell scripts (e.g. an autostart hook).
- **Discard** — "close tab" for sessions: switch back to your previous session, then soft-kill the one you left.
- **Fast** — a single sync Rust binary (~1.2 MB, 7 crates, no TUI framework, no async runtime); `list` in ~20 ms (~250 ms with the attached indicator).
- **Shell completions** — fish/zsh/bash/nushell, with live session-name completion.

## Install

### Nix flake (recommended)

```nix
# flake.nix
inputs.noren = {
  url = "github:MaySeikatsu/noren";   # or git+file:///path/to/checkout
  inputs.nixpkgs.follows = "nixpkgs";
};
```

`packages.<system>.default` is the wrapped binary (runtime deps on PATH). See **[docs/integration.md](docs/integration.md)** for the full home-manager wiring: keybinds, shell bindings, completions, autostart.

### Cargo

```sh
cargo install --path .
```

Runtime dependencies on PATH: `zellij` (≥ 0.42), `zoxide`, `fzf`, `git`; optional: `eza` (nicer directory previews). For in-place switching inside zellij, install [zellij-switch.wasm](https://github.com/mostafaqanbaryan/zellij-switch) to `~/.config/zellij/plugins/`.

## Usage

```
noren                              interactive picker (default)
noren <name-or-path>               connect shorthand (sesh-style)
noren connect <name-or-path>       attach / create / resurrect / switch
noren last                         jump back to the previous session
noren root [path]                  connect to the git top-level session
noren list [zellij|zoxide|config|all|blacklist]
noren kill <name>                  soft kill (layout survives)
noren delete <name>                hard delete (confirms on a tty)
noren discard [name]               switch to previous session, then soft-kill
noren mkdir <path>                 mkdir -p + connect
noren clone <git-url> [dest]       git clone + connect
noren pin [name]                   toggle pin (default: current session)
noren rename <new>                 rename current session, carry pin over
noren snapshot [name]              save a point-in-time layout backup
noren snapshots [name]             list backups, newest first
noren restore <name> [i] [--force] recreate from backup i (1 = newest)
noren window [target] [-s session] list / switch / create tabs
noren name-for <path>              session name a path resolves to
noren resolve <target> [--format env|json]
```

### Picker keys

| Key | Action |
| --- | --- |
| `Enter` | connect |
| `Ctrl t` / `Ctrl z` / `Ctrl c` / `Ctrl a` | filter: sessions / dirs / config / all |
| `Ctrl r` | reload |
| `Ctrl p` | pin toggle (sessions by name, dirs by path) |
| `Ctrl d` | soft-kill selected |
| `Ctrl Alt d` | hard-delete selected |
| `Esc` | close |

## Configuration

`~/.config/noren/config.toml` — everything is optional; see [config.toml.example](config.toml.example) for the annotated full set.

```toml
sort_order    = ["zellij", "config", "zoxide"]   # first group = closest to the prompt
session_order = ["pinned", "live", "exited"]
separator     = true          # hairline between sessions and folders
preview_mode  = "visual"      # or "text"
snapshot_keep = 5
auto_snapshot_pinned = false
attached_indicator   = true   # ▣/▢ (costs ~200ms; false = instant list)

[default_session]
preview_command = "eza --all --git --icons --color=always {}"

[[session]]                   # explicit project — highest resolver precedence
name            = "nixos"
path            = "~/.config/nixos"
layout          = "ide-git"
startup_command = "hx"

[[wildcard]]                  # glob rules; * = one segment, ** = recursive
pattern = "~/Projects/rust/*"
layout  = "ide"
startup_command = "hx ."
```

Resolver precedence for `noren <target>`: `[[session]]` (name or path) → live/exited session by name → `[[wildcard]]` on an existing path → sanitized fallback name.

## File locations

| Path | Purpose |
| --- | --- |
| `~/.config/noren/config.toml` | configuration (legacy `~/.config/zjp/config.toml` read as fallback) |
| `~/.local/state/zjp/last`, `previous` | connect history (powers `noren last`) |
| `~/.local/state/zjp/pinned-dirs` | pinned folder paths (noren-owned) |
| `~/.local/state/zjp/snapshots/<session>/` | layout snapshots |
| `~/.local/state/zellij/pinned` | pinned session names — deliberately shared, so external tools (status bars, reapers) can honor pins |
| `~/.cache/zellij/…/session_info/` | zellij's own layout serialization (read-only for previews/snapshots) |

## Integration

Zellij keybinds, home-manager module examples, shell bindings, completions, and the autostart hook are documented in **[docs/integration.md](docs/integration.md)**.

## Design notes

- **No TUI framework.** fzf is the UI; noren feeds it TSV and re-execs itself for reloads, previews, and actions. Rust startup is ~milliseconds, so this costs nothing and keeps the binary tiny.
- **Previews never talk to the zellij server.** Both preview modes render from the serialized layout on disk — a dead session cannot be resurrected by scrolling past it.
- **Snapshots capture what zellij serializes** — layout, cwds, running commands — not in-program state. Restore relaunches the commands.
- **Multi-client caveat:** when two terminals show the same session, switching from inside moves the *oldest* attached client (zellij CLI calls carry no client identity). The ▣ indicator exists so you see shared sessions before joining them.

## Inspiration & similar projects

- **[sesh](https://github.com/joshmedeski/sesh)** by Josh Medeski — the blueprint. noren started as "sesh, but for zellij" and still follows its CLI shape and philosophy.
- **[zoxide](https://github.com/ajeetdsouza/zoxide)** and **[fzf](https://github.com/junegunn/fzf)** — the two tools doing the heavy lifting: frecent directories in, fuzzy selection out.
- **[zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch)** by Mostafa Qanbaryan — the WASM plugin that makes in-place session switching possible at all.

If noren isn't your flavor, these solve the same problem differently:

- [zesh](https://github.com/roberte777/zesh) — zellij + zoxide session manager, also sesh-inspired, Rust.
- [zellij-sessionizer](https://github.com/cunialino/zellij-sessionizer) — minimal directory-based sessionizer.
- [zism](https://github.com/23prime/zism) — zellij interactive session manager.
- zellij's built-in session-manager plugin (`Ctrl o w`) — no install, fewer sources, but as a resident plugin it can do per-client switching, which no external CLI can.

## Thanks

Thanks to everyone who files issues, sends PRs, or tests noren on their setup — contributions of any size are welcome. Special thanks to the [zellij](https://github.com/zellij-org/zellij) maintainers for the serialization and pipe machinery this tool leans on.

## License

[MIT](LICENSE)
