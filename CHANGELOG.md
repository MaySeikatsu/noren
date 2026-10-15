# Release notes

## 0.2.2

**Tab menu.** `Alt t` on a session opens its tabs: name, focused program and
directory, the active tab marked `●`; the preview shows that tab's live
screen (or the saved layout for exited sessions, which are never
contacted). `⏎` connects and lands in that tab — inside zellij the client
switches and then moves to the tab; from a plain terminal noren attaches
and a small helper moves the new client once it has arrived. `Esc` returns
to the previous view.

## 0.2.1

**Snapshot menu in the picker.** `Ctrl s` on a session now opens its
snapshots instead of silently taking one:

- `＋ take a snapshot of <session> now` sits next to the prompt; `⏎` on it
  (or `Ctrl s` anywhere in the menu) saves one.
- Saved snapshots are listed newest first with time, age and tab count; the
  preview shows the layout each one holds.
- `⏎` on a snapshot restores it after an inline `y/N` — the current state is
  snapshotted first, so a restore can itself be undone. Restoring the session
  you are in is refused with an explanation.
- `Ctrl d` deletes a snapshot (after `y/N`); `Esc` returns to the view you
  came from.

**Hide entries.** `Ctrl h` hides the selected session, folder or config
entry from the list (stored in `~/.local/state/noren/hidden`, separate from
the config `blacklist`); `Alt h` shows hidden entries — marked `⊘`, dimmed —
so `Ctrl h` can unhide them. `noren list hidden` lists them. Entries hidden
by the config blacklist explain where to unhide them.

**Rename from the picker.** `Ctrl e` turns the prompt into an input
prefilled with the selected session's name; `⏎` renames (pin, history,
snapshots and hidden state move along), `Esc` cancels. Exited sessions
can't be renamed (zellij limitation) and say so.

**Typed paths.** A query that looks like a path (`~/…`, `/…`, `./…`) means
that folder: `⏎` connects to it, or — if it doesn't exist — asks
`create ~/x and a session there? y/N`.

**Search paths too.** `Alt p` switches matching between names only and
names + paths.

**`↩` marks the `noren last` target** in the list.

**Key overview wraps** to the width of the list pane (and rewraps on
resize), so binds are no longer cut off in narrow or floating panes.

Fixes: snapshot tab counts no longer include tab templates; the live
preview could show another running session when the previewed one had
just exited (zellij answers for any running session when the requested
one is gone) — it now checks first.

## 0.2.0

**Live previews.** Running sessions show what their focused pane displays
right now, with a tab strip, refreshed every second while you look
(`live_refresh_ms`). `Ctrl l` toggles live ↔ layout; the preview label shows
which. Exited sessions are never contacted.

**Picker**

- New default keys: `Ctrl f` config view (was `Ctrl c`), `Ctrl k` kill,
  `Ctrl d` delete (was `Ctrl Alt d`), `Ctrl s` snapshots, `Ctrl /` preview,
  `Ctrl l` live/layout. All configurable under `[keys]`; the header lists
  exactly the active binds.
- Kill and delete ask `y/N` inline in the prompt, per `[confirm]`
  (default: kill asks when a terminal is attached, delete when the session
  is running). Killing or deleting the session you are in switches you away
  first.
- Reload, pin, kill and delete keep the current view.
- `⏎` with no match creates a session named after the query; `⏎` on the
  separator does nothing.
- Typing ranks matches (best next to the prompt) and matches names only;
  each row shows its path dimmed.
- Status messages appear in the empty query line.
- Layout: preview beside the list when roomy, above it when narrow
  (`preview_position`, `preview_position_compact`).
- Config problems are shown in the header.

**Fixes**

- Picking a session connected to a different session when its first tab's
  directory had another name (also after renames).
- Folders sharing a basename hid each other; blacklisting a name hid every
  folder with that name (folders now match by path or glob).
- Switching inside zellij could do nothing silently (missing or
  unpermitted zellij-switch plugin); it now reports why.
- Sessions with a layout created from inside zellij came up empty on
  zellij 0.45.
- `preview_command` paths are shell-quoted (`$(…)` in a folder name no
  longer runs).
- `noren rename` carries `noren last` history and snapshots; discard and
  delete no longer leave a closed session as the `noren last` target.
- Snapshots of live sessions can no longer hang on an unresponsive server.
- Config errors report line and column; unknown keys are warned about.

**Other**

- Session previews start with a "last active" line (tab · program · cwd).
- `noren kill|delete --yes`, `noren --version`.
- Fixed waits replaced by readiness polling (faster connect/discard/restore).

## 0.1.0

First public release.
