# noren (暖簾)

<div align="center">
   <img width="128" alt="noren" src="https://github.com/user-attachments/assets/4316da74-685f-4d22-8dfc-9faeeac7e78e" />
</div>

##

**A sesh-style session manager for [zellij](https://zellij.dev). Brush through the curtain into any project.**

A *noren* is the fabric curtain hanging in the doorway of a Japanese shop — you don't open it, you brush through it without breaking stride. That's the feel this tool is after: one keypress, a fuzzy match, and you're standing in another project.


noren merges your **running zellij sessions**, **[zoxide](https://github.com/ajeetdsouza/zoxide) directories**, and **configured project entries** into a single [fzf](https://github.com/junegunn/fzf) picker, and connects to whatever you choose — attaching, creating, resurrecting, or switching in place as appropriate. Feature parity with [sesh](https://github.com/joshmedeski/sesh) (tmux), plus a few things sesh doesn't have.
```
   📁 rust/tutorials   ~/Projects/rust/tutorials            ╭──── preview · ● live · all tabs ─────╮
   ⚙ dotfiles          ~/dotfiles                           │ ● 2 tabs · nixos                     │
   ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌                                       │ ● code           ┃   rebuild         │
   ⊗ old-experiment    ~/Projects/old                       │fn main() {       ┃building…          │
   ▣ projects ↩        ~/Projects                           │    rebuild();    ┃copying path       │
   📌 ▢ quickshell (bar, logs)  ~/.config/quickshell        │}                 ┃done.              │
   📌 ▣ nixos (code, rebuild)   ~/.config/nixos             │:w                ┃$                  │
                                                            │                  ┃                   │
  ^t sessions  ^z dirs  ^w tabs  ⏎ connect     f1 more      │                                      │
⚡  nix▏                                                    ╰──────────────────────────────────────╯
```

📌 pinned · ▣ open in a terminal right now · ▢ running in the background · ⊗ exited (resurrectable) · ⚙ config entry · 📁 zoxide folder · ↩ where `noren last` jumps back to.

## Features

**Finding things**
- **One picker, three sources** — live/exited sessions, zoxide dirs, `[[session]]` config entries; deduplicated by identity (sessions by name, folders by path), sorted so the most relevant row sits next to the prompt, with a dim hairline between sessions and folders.
- **Fuzzy ranking on names** — typing ranks matches, best next to the prompt; `Alt p` searches paths too. Each row shows its path dimmed; the session's tab names follow the name (`show_tabs`).
- **Typed paths & new sessions** — no match + `Enter` creates a session named after the query; a path (`~/x`, `/x`, `./x`) opens that folder, offering to create it first.
- **Hide entries** — `Ctrl h` hides a session, folder or config entry; `Alt h` shows hidden ones (⊘) to unhide them. For permanent rules (`**/node_modules`, `/nix/store/**`) use `blacklist` in the config.
- **Stays current** — the list re-lists itself every minute while open (`auto_reload_secs`), keeping the cursor on its entry.

**Seeing what's there**
- **Live previews** — a running session's preview shows *right now* what all its tabs display, side by side (or just the active tab), refreshed every second. `Ctrl /` cycles all tabs → active tab → layout → text → off (or, with `preview_toggle = "separate"`, only shows/hides while `Ctrl l` cycles the style). The preview's title always says what it shows (`● live · all tabs`, `▦ layout — session not running`, …), and its background is a shade off the terminal's so picker and preview read as two areas.
- **Layout previews without resurrection** — the pane layout drawn as box art (focused pane highlighted, running programs labeled), rendered *only* from zellij's saved layout. Exited sessions are never contacted, so hovering one can never wake it.
- **Attached indicator** — ▣ marks sessions shown in a terminal right now, ▢ ones running in the background.

**Acting on it**
- **Session-aware connect** — inside zellij it switches your client in place (via the [zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch) plugin — never nested attaches); outside it execs `zellij attach --create`. `Alt Enter` opens the selection in a new terminal window instead.
- **Kill, delete, rename, pin — from the picker** — inline `y/N` confirmations right in the prompt (configurable per action), rename with the name prefilled; killing the session you're in switches you away first.
- **Tab menu** — `Ctrl w` lists a session's tabs (live preview of each, the active one marked ●); `Enter` takes you straight into that tab, from inside zellij or a fresh terminal.
- **Snapshots** — point-in-time backups of a session's layout + running commands. `Ctrl s` opens a session's snapshot menu: take one, preview any of the last N, restore or delete it. Optionally automatic for pinned sessions.
- **Alt-tab for sessions** — `noren last` toggles current ↔ previous in place; bound to `Alt Shift Tab` it feels like window switching (see [docs/integration.md](docs/integration.md)).
- **Discard** — "close tab" for sessions: switch back to your previous session, then soft-kill the one you left.
- **Project rules** — `[[session]]` entries and `[[wildcard]]` globs assign layouts, startup commands, and preview commands per project; `noren resolve` exposes the same resolver to shell scripts (e.g. an autostart hook).

**Under the hood**
- **Configurable everything** — every key under `[keys]`; a compact one-line key overview (`header_lines`) with `F1` for the full list of every bind; config problems are shown with line and column.
- **Fast** — a single sync Rust binary (7 crates, no TUI framework, no async runtime); `list` in ~20 ms (~250 ms with the attached indicator).
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

`packages.<system>.default` is the wrapped binary (runtime deps on PATH). Release notes: **[CHANGELOG.md](CHANGELOG.md)**. See **[docs/integration.md](docs/integration.md)** for the full home-manager wiring: keybinds, shell bindings, completions, autostart.

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
noren --version
noren kill <name> [--yes]          soft kill (layout survives; asks if attached)
noren delete <name> [--yes]        hard delete (asks if running)
noren discard [name]               switch to previous session, then soft-kill
noren mkdir <path>                 mkdir -p + connect
noren clone <git-url> [dest]       git clone + connect
noren pin [name]                   toggle pin (default: current session)
noren rename <new>                 rename current session (pin, history, snapshots move along)
noren snapshot [name]              save a point-in-time layout backup
noren snapshots [name]             list backups, newest first
noren restore <name> [i] [--force] recreate from backup i (1 = newest)
noren window [target] [-s session] list / switch / create tabs
noren name-for <path>              session name a path resolves to
noren resolve <target> [--format env|json]
```

### Picker keys

The overview above the prompt shows the main keys; `F1` toggles the full list.

| Key | Action |
| --- | --- |
| `Enter` | connect — no match: create a session named after the query; a typed path goes to that folder (offering to create it) |
| `Alt Enter` | open the selection in a new terminal window (`terminal` in the config, else `$TERMINAL`) |
| `Esc` | close (or cancel a pending confirmation / rename / menu) |
| `Ctrl t` / `Ctrl z` / `Ctrl a` | view: sessions / folders / all |
| `Ctrl f` | view: `[[session]]` entries from the config (listed only if you have some) |
| `Ctrl r` | reload the current view |
| `Ctrl /` | cycle the preview: all tabs → active tab → layout → text → off |
| `Ctrl l` | cycle the preview style only (with `preview_toggle = "separate"`, `Ctrl /` just shows/hides) |
| `Ctrl p` | pin / unpin (sessions by name, folders by path) |
| `Ctrl e` | rename the selected running session (`Enter` applies, `Esc` cancels) |
| `Ctrl k` | soft-kill (layout survives, resurrectable) |
| `Ctrl d` | hard-delete (removes the saved layout) |
| `Ctrl w` | tab menu of the session: its tabs with a live preview each; `Enter` lands in that tab, `Esc` back |
| `Ctrl s` | snapshot menu: `Enter` restore, `Ctrl s` take one, `Ctrl d` delete, `Esc` back |
| `Ctrl h` | hide / unhide the selected entry |
| `Alt h` | show hidden entries (⊘) or not |
| `Alt p` | search names only ↔ names + paths |
| `F1` | full ↔ compact key overview |

Kill and delete ask `y/N` in the prompt line when the `[confirm]` policy says so (default: kill asks if a terminal is attached, delete asks if the session is running; exited sessions are deleted directly). All binds are configurable under `[keys]` and override fzf's own defaults inside noren's picker only — e.g. `Ctrl h`, which a few terminals send for Backspace; rebind `hide` if yours does. `Ctrl Enter` can't be used for the new window: most terminals send the same code as `Enter`.

## Configuration

`~/.config/noren/config.toml` — everything is optional; see [config.toml.example](config.toml.example) for the annotated full set.

```toml
sort_order    = ["zellij", "config", "zoxide"]   # first group = closest to the prompt
session_order = ["pinned", "live", "exited"]
separator     = true          # hairline between sessions and folders
show_paths    = true          # dimmed path after each name
show_tabs     = "inline"      # session tab names behind the name; "column" / "off"
blacklist     = ["/nix/store/**", "**/node_modules"]   # names, paths or globs

preview_mode  = "live"        # live screen of running sessions; or "visual" / "text"
live_all_tabs = true          # live preview of every tab side by side
preview_toggle = "combined"   # or "separate": ^/ shows/hides, ^l cycles the style
preview_tint  = true          # preview background a shade off the terminal's
live_refresh_ms  = 1000       # live preview refresh while you look
auto_reload_secs = 60         # re-list while the picker is open (0 = off)
preview_position         = "right"   # beside the list when roomy…
preview_position_compact = "up"      # …above it when narrow
header_lines  = 1             # compact key overview (F1 shows all)
terminal      = ""            # for Alt Enter; "" = $TERMINAL, or "foot", or "footclient {}"

attached_indicator   = true   # ▣/▢ (costs ~200ms; false = instant list)
snapshot_keep        = 5
auto_snapshot_pinned = false

[keys]                        # fzf key names; "" disables a bind
kill   = "ctrl-k"
delete = "ctrl-d"
new_window = "alt-enter"

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
| `~/.local/state/noren/hidden` | entries hidden with `Ctrl h` |
| `~/.local/state/noren/snapshots/<session>/` | layout snapshots |
| `~/.local/state/zellij/pinned` | pinned session names — deliberately shared, so external tools (status bars, reapers) can honor pins |
| `~/.cache/zellij/…/session_info/` | zellij's own layout serialization (read-only for previews/snapshots) |

Pre-rename files under `~/.config/zjp/` and `~/.local/state/zjp/` are copied to the noren locations automatically on first run (copied, not moved — the predecessor tools keep working).

## Integration

Zellij keybinds, home-manager module examples, shell bindings, completions, and the autostart hook are documented in **[docs/integration.md](docs/integration.md)**.

## Design notes

- **No TUI framework.** fzf is the UI; noren feeds it TSV and re-execs itself for reloads, previews, and actions. Rust startup is ~milliseconds, so this costs nothing and keeps the binary tiny.
- **Previews never wake a dead session.** Layout previews render from the serialized layout on disk; the live preview only contacts sessions zellij lists as running (it double-checks — zellij answers for *any* running session when asked about a missing one), under a short timeout, and falls back to the layout render on any failure.
- **fzf's socket** (`--listen`, a unix socket in `$XDG_RUNTIME_DIR`) lets previews label themselves and drives the live refresh and auto-reload.
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
