# noren (暖簾)

<div align="center">
   <img width="128" alt="noren" src="https://github.com/user-attachments/assets/4316da74-685f-4d22-8dfc-9faeeac7e78e" />
</div>

##

**A sesh-style session manager for [zellij](https://zellij.dev). Brush through the curtain into any project.**

A *noren* is the fabric curtain hanging in the doorway of a Japanese shop — you don't open it, you brush through it without breaking stride. That's the feel this tool is after: one keypress, a fuzzy match, and you're standing in another project.


noren merges your **running zellij sessions**, **[zoxide](https://github.com/ajeetdsouza/zoxide) directories**, and **configured project entries** into a single [fzf](https://github.com/junegunn/fzf) picker, and connects to whatever you choose — attaching, creating, resurrecting, or switching in place as appropriate. Feature parity with [sesh](https://github.com/joshmedeski/sesh) (tmux), plus a few things sesh doesn't have.
```
⚡  nixos▏
   📌 ▣ nixos          ← pinned, open in a terminal right now   ┌─ preview ────────────┐
   📌 ▢ quickshell  ← pinned, running in the background         │ ┏━━━━━━━┓┌─────────┐ │
   ▣ projects                                                   │ ┃ hx  * ┃│ cargo   │ │
   ⊗ old-experiment    ← dead, resurrectable                    │ ┗━━━━━━━┛└─────────┘ │
   ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌                                           │ (+ 1 floating)       │
   ⚙ dotfiles          ← from config.toml                       └──────────────────────┘
   📁 rust/tutorials   ← from zoxide
```

## Features

- **One picker, three sources** — live/exited sessions, zoxide dirs, `[[session]]` config entries; deduplicated (sessions beat config beat dirs), sorted so the most relevant row sits next to the prompt, with a dim hairline separating sessions from folders.
- **Session-aware everywhere** — inside zellij it switches your client in place (via the [zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch) plugin — never nested attaches); outside it exec's `zellij attach --create`.
- **Live previews** — a running session's preview shows what its focused pane displays *right now*, refreshed while you look; `Ctrl l` flips to the layout view.
- **Layout previews without resurrection** — the pane layout drawn as box art (focused pane highlighted, running programs labeled), rendered *only* from zellij's serialized layout cache. Exited sessions are never contacted, so hovering one can never wake it.
- **Attached indicator** — ▣ marks sessions currently shown in a terminal, ▢ ones running in the background.
- **Pinning** — pin sessions (shared, name-based) and folders (separate, path-based); pinned rows float to the prompt.
- **Snapshots** — point-in-time backups of a session's layout + running commands. `Ctrl s` opens a session's snapshot menu: take one, preview any of the last N, restore or delete it. Optionally automatic for pinned sessions.
- **Project rules** — `[[session]]` entries and `[[wildcard]]` globs assign layouts, startup commands, and preview commands per project; `noren resolve` exposes the same resolver to shell scripts (e.g. an autostart hook).
- **Discard** — "close tab" for sessions: switch back to your previous session, then soft-kill the one you left.
- **Alt-tab for sessions** — `noren last` toggles current ↔ previous in place; bound to `Alt Shift Tab` it feels like window switching (see [docs/integration.md](docs/integration.md)).
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

Runtime dependencies on PATH: `zellij` (≥ 0.42; verified on 0.45), `zoxide`, `fzf` (verified on 0.74 — the picker uses `transform`, ghost text and `accept-or-print-query`), `git`; optional: `eza` (nicer directory previews). For in-place switching inside zellij, install [zellij-switch.wasm](https://github.com/mostafaqanbaryan/zellij-switch) to `~/.config/zellij/plugins/` and grant its permissions once — it is required: zellij's own `action switch-session` does not move clients when called from the CLI (0.45), and noren reports an error instead of silently doing nothing.

## Usage

```
noren                              interactive picker (default)
noren <name-or-path>               connect shorthand (sesh-style)
noren connect <name-or-path>       attach / create / resurrect / switch
noren last                         jump back to the previous session
noren root [path]                  connect to the git top-level session
noren list [zellij|zoxide|config|all|hidden]
noren kill <name> [--yes]          soft kill (layout survives; asks if attached)
noren delete <name> [--yes]        hard delete (asks if running)
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
| `Enter` | connect — with no match, create a session named after the query; a typed path (`~/x`, `/x`, `./x`) goes to that folder, offering to create it first |
| `Ctrl t` / `Ctrl z` / `Ctrl f` / `Ctrl a` | view: sessions / dirs / config / all |
| `Ctrl r` | reload the current view |
| `Ctrl p` | pin toggle (sessions by name, dirs by path) |
| `Ctrl s` | snapshot menu of the selected session (⏎ restore, `Ctrl s` take one, `Ctrl d` delete, `Esc` back) |
| `Ctrl k` | soft-kill the selected session |
| `Ctrl d` | hard-delete the selected session |
| `Ctrl /` | show / hide the preview |
| `Ctrl l` | session previews: live screen ↔ saved layout |
| `Ctrl h` | hide / unhide the selected session, folder or config entry |
| `Alt h` | show hidden entries (marked ⊘) in the list, or not |
| `Ctrl e` | rename the selected running session (type the name, `Enter` applies, `Esc` cancels) |
| `Alt p` | search names only (default) or names + paths |
| `Esc` | close (or cancel a pending confirmation) |

`↩` marks the session `noren last` would jump back to. All binds are configurable under `[keys]`; the header inside the picker is generated from the active set and wraps to the pane width, so no bind is ever cut off. noren's binds override fzf's defaults inside its picker only (e.g. `Ctrl h`, which some terminals send for Backspace — rebind `hide` if yours does). Kill and delete ask `y/N` right in the prompt line when the `[confirm]` policy says so (default: kill asks if a terminal is attached, delete asks if the session is running); killing or deleting the session you are in switches you away first.

## Configuration

`~/.config/noren/config.toml` — everything is optional; see [config.toml.example](config.toml.example) for the annotated full set.

```toml
sort_order    = ["zellij", "config", "zoxide"]   # first group = closest to the prompt
session_order = ["pinned", "live", "exited"]
separator     = true          # hairline between sessions and folders
preview_mode  = "live"        # live screen of running sessions; or "visual" / "text"
preview_position         = "right"   # beside the list when roomy…
preview_position_compact = "up"      # …above it when narrow
snapshot_keep = 5
auto_snapshot_pinned = false
attached_indicator   = true   # ▣/▢ (costs ~200ms; false = instant list)
show_paths           = true   # dimmed path after each name

[keys]                        # fzf key names; "" disables a bind
config = "ctrl-f"
kill   = "ctrl-k"
delete = "ctrl-d"

[confirm]                     # always | attached | running | never
kill   = "attached"
delete = "running"

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
| `~/.config/noren/config.toml` | configuration |
| `~/.local/state/noren/last`, `previous` | connect history (powers `noren last` / the alt-tab toggle) |
| `~/.local/state/noren/pinned-dirs` | pinned folder paths (noren-owned) |
| `~/.local/state/noren/snapshots/<session>/` | layout snapshots |
| `~/.local/state/zellij/pinned` | pinned session names — deliberately shared, so external tools (status bars, reapers) can honor pins |
| `~/.cache/zellij/…/session_info/` | zellij's own layout serialization (read-only for previews/snapshots) |

Pre-rename files under `~/.config/zjp/` and `~/.local/state/zjp/` are copied to the noren locations automatically on first run (copied, not moved — the predecessor tools keep working).

## Integration

Zellij keybinds, home-manager module examples, shell bindings, completions, and the autostart hook are documented in **[docs/integration.md](docs/integration.md)**.

## Design notes

- **No TUI framework.** fzf is the UI; noren feeds it TSV and re-execs itself for reloads, previews, and actions. Rust startup is ~milliseconds, so this costs nothing and keeps the binary tiny.
- **Previews never wake a dead session.** Layout previews render from the serialized layout on disk; the live preview only contacts sessions listed as running, under a short timeout, and falls back to the layout render on any failure.
- **Snapshots capture what zellij serializes** — layout, cwds, running commands — not in-program state. Restore relaunches the commands.
- **Multi-client caveat:** when two terminals show the same session, switching from inside moves the *oldest* attached client (zellij CLI calls carry no client identity). The ▣ indicator exists so you see shared sessions before joining them.

## AI use

AI was used to realise this project - two basic local prototypes were created previously that were nushell+python based hence the original "zjp3" naming. I'm happy you got here and found the project, please be aware that it was created as personal tooling and might contain bugs.

## Thanks

Thanks to everyone who files issues, sends PRs, or tests noren on their setup — contributions of any size are welcome. Special thanks to the [zellij](https://github.com/zellij-org/zellij) maintainers for the serialization and pipe machinery this tool leans on. And zoxide + fzf for building the foundation for this tool:
- **[zoxide](https://github.com/ajeetdsouza/zoxide)** and **[fzf](https://github.com/junegunn/fzf)** — the two tools doing the heavy lifting: frecent directories in, fuzzy selection out.
- **[zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch)** by Mostafa Qanbaryan — the WASM plugin that makes in-place session switching possible at all.

### Inspiration & similar projects

- **[sesh](https://github.com/joshmedeski/sesh)** by Josh Medeski — the blueprint. noren started as "sesh, but for zellij" and still follows its CLI shape and philosophy.
- **[zesh](https://github.com/roberte777/zesh)** — zellij + zoxide session manager, also sesh-inspired, Rust.
- **[zellij-sessionizer](https://github.com/cunialino/zellij-sessionizer)** — minimal directory-based sessionizer.
- **[zism](https://github.com/23prime/zism)** — zellij interactive session manager.
- **zellij's built-in session-manager** plugin (`Ctrl o w`) — no install, fewer sources, but as a resident plugin it can do per-client switching, which no external CLI can.


## License

[MIT](LICENSE)
