# Roadmap & ideas

Living document: review findings, planned work, and design brainstorms that
are not (yet) features. Shipped items move to the changelog section at the
bottom with the commit that landed them.

## 1. Review findings (2026-10) — fix wave `feat/ux-overhaul`

### Bugs

| # | Problem | Planned fix |
|---|---------|-------------|
| 1 | Picker connects session rows by their *path* (first `cwd` in the layout cache), so `cubic-bee` with cwd `~/.config/nixos` attaches to a session named `nixos` instead. Renamed sessions break the same way. | Session rows always connect by name; paths only for dir/config rows. |
| 2 | `dedupe` and `blacklist` key on the display name — dirs sharing a basename hide each other (371 zoxide dirs → 340 shown), blacklisting `scratch` hides every `scratch/` folder. | Sessions dedupe by name, dirs by path; cross-source only when paths are equal. |
| 3 | `^r`/`^p`/`^d` reload `list all` without changing the prompt — the filter shown and the rows listed disagree after `^t`. | Reload the view named by `$FZF_PROMPT`. |
| 4 | `switch_in_place` ignores the exit status — a missing `zellij-switch.wasm` makes Enter silently do nothing. | Check status, fall back to `zellij action switch-session`, error with an install hint. |
| 5 | Kill/delete semantics are confusing; `^d` on the current session drops the client; `^alt-d` on a live session fails invisibly (needs `--force`). | `^k` kill, `^d` delete, configurable confirmation policy (attached / running / always / never), current session routes through discard. |
| 6 | `preview_command` substitutes `{}` unquoted — spaces break, `$(…)` in a dir name executes. | Shell-quote the path; session previews show focused program + cwd. |
| 7 | `rename` moves the pin but not `last`/`previous` history or the snapshot dir — `noren last` then creates an empty session under the old name. | Rename carries history + snapshots. |
| 8 | `take_snapshot` runs `dump-layout` without a timeout (on the connect path with `auto_snapshot_pinned`). | Reuse the kill-timeout helper. |
| 9 | README logo `<img>` tag malformed. | Fix markup. |
| 10 | Config parse errors lack line/column; unknown keys are silently ignored. | Report toml error detail, warn on unknown keys (also inside the picker header). |

### UX improvements (approved)

- Fuzzy ranking while typing: drop `--no-sort`, use `--tiebreak=index` (empty query keeps the curated order).
- Dimmed path after each name — disambiguates random session names and same-basename dirs.
- Enter on no match creates a session from the query (`accept-or-print-query`).
- Enter on the separator row does nothing instead of closing the picker.
- Responsive preview window (moves below the list in narrow panes).
- Poll for session readiness instead of fixed 300 ms sleeps.
- Native `zellij action switch-session` as a fallback to the wasm plugin.
- `record_last` uses `$ZELLIJ_SESSION_NAME` so `noren last` survives switches made by other tools.
- Configurable keybinds (`[keys]`), config filter moves from `^c` to `^f`, header generated from the active binds so every bind is visible.

## 2. noren-tui — embedded live session preview (deferred)

Goal: a picker that shows what a session actually looks like right now, not
just its layout skeleton.

### Primitives available in zellij 0.45

| Command | Gives |
|---------|-------|
| `zellij watch <session>` | read-only attached client (full rendered UI) |
| `zellij --session X subscribe -p terminal_1 --ansi [--format json]` | streaming render updates of specific panes |
| `zellij action dump-screen -p <id> --ansi` | one-shot pane viewport |
| `zellij action list-panes -a -j` | pane geometry, focus, commands, tabs (JSON) |

All of them must only ever target **live** sessions, under a kill-timeout
(`zellij action` against exited sessions hangs; a session dying mid-call
hangs too).

### Options, cheapest first

| | Approach | Gains | Cost / risk |
|---|---|---|---|
| A | Stay on fzf; preview command = `dump-screen`/`subscribe` of the focused pane for live sessions | real, updating content; ~80 % of the value | tiny; fzf already kills the preview process on cursor move |
| B | ratatui composite: geometry from `list-panes`, each box filled with its own subscribed stream (`ansi-to-tui` / `vt100`) | all panes at once in the existing box-art layout; multi-session grid ("exposé") | medium; content can only be cropped, not scaled |
| C | True embed: `zellij watch X` in a pty (`portable-pty` + `vt100` + `tui-term`) | pixel-exact, incl. frames and status bar | per-hover client spawn (debounce); **verify** whether a watch client participates in size negotiation (could shrink the real session); may need `ZELLIJ` env removed when nested |
| D | WASM plugin | per-client switching | plugins get other sessions' *metadata* only — no pane content, so no embed |

### Recommendation

1. Spike option A (≈1 h): measure `subscribe` latency, behavior when the session dies, use from outside zellij.
2. If a TUI is built: `noren tui` behind a cargo feature (default build keeps its small dep set); move `sources` / `resolve` / `connect` into `lib.rs`.
3. Architecture: crossterm event loop; one preview worker per hovered session with cancel-on-move and ~100 ms debounce; last-frame cache per session; `nucleo` for fuzzy matching.
4. Build B first (reuses the layout renderer); C as an experiment once size negotiation is understood.
5. A TUI does not change the multi-client limitation — CLI switching still moves the oldest attached client.

## 3. To investigate: restore fidelity of program state

`noren restore` (and zellij resurrection in general) relaunches the
serialized *commands* (`hx`, `nvim`, …) in their cwds, but not in-program
state — open buffers, cursor positions, splits inside the editor. Observed:
helix/neovim buffers are not always restored. Directions:

- What zellij serializes: `command` + `args` of a pane at serialization time (check whether file arguments survive, e.g. `hx src/main.rs` vs plain `hx`); `serialize_pane_viewport` only stores scrollback text.
- Editor-side session persistence: neovim `:mksession` / `persistence.nvim` / `auto-session` keyed by cwd; helix has no native sessions yet (community workarounds only).
- noren-side hook: optional `restore_command` per `[[session]]`/`[[wildcard]]` (e.g. `nvim -c 'lua require("persistence").load()'`) substituted for the bare editor command when restoring from a snapshot.
- Snapshot metadata: record focused file per editor pane where discoverable (e.g. from the pane title) to pass as an argument on restore.

## Changelog

_(filled as items land)_
