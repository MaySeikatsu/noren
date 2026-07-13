# Integrating noren

How to wire noren into a system properly: NixOS/home-manager, zellij keybinds,
shell bindings, completions, and the autostart hook. Everything here is
optional — the binary works standalone — but this is the full setup the tool
is designed around.

## 1. Nix flake input

```nix
# flake.nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    noren = {
      url = "github:MaySeikatsu/noren";          # or git+file:///path/to/checkout
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
```

The package is `inputs.noren.packages.<system>.default`. It is a
`buildRustPackage` wrapped with all runtime dependencies on PATH
(`zellij`, `zoxide`, `fzf`, `git`, `eza`, coreutils) and ships shell
completions for fish/zsh/bash in the standard vendor directories (picked up
automatically by home-manager-enabled shells) plus a nushell completion file
at `share/nushell/noren-completions.nu`.

> **Local-checkout inputs:** a `git+file://` URL pins a *commit*, not the
> working tree. After changing noren: commit, then `nix flake update noren`,
> then rebuild.

## 2. Home-manager module (full example)

```nix
{ pkgs, lib, inputs, ... }: let
  noren = inputs.noren.packages.${pkgs.stdenv.hostPlatform.system}.default;
in {
  home.packages = [ noren ];

  # Universal short alias — works in any shell/terminal.
  home.shellAliases.zs = "noren";

  # Ship the annotated example config next to the real one.
  home.file.".config/noren/config.toml.example".source =
    "${inputs.noren}/config.toml.example";

  # Alt+Shift+S at the shell prompt opens the picker — this covers bare
  # terminals, TTYs, and SSH. Inside zellij the zellij bind (below) consumes
  # the chord first; in zellij's locked mode the key passes through and these
  # fire instead. Both paths are safe: noren switches in place when inside.
  programs.fish.interactiveShellInit = ''
    bind \eS 'noren; commandline -f repaint'
  '';

  programs.zsh.initContent = ''
    _noren-picker() { zle -I; noren </dev/tty; zle reset-prompt }
    zle -N _noren-picker
    bindkey '\eS' _noren-picker
  '';

  # Nushell: completions have no vendor-dir convention, so source explicitly.
  # If your config assigns `$env.config = {…}` elsewhere, order this after it
  # (lib.mkAfter).
  programs.nushell.extraConfig = lib.mkAfter ''
    source ${noren}/share/nushell/noren-completions.nu
    $env.config.keybindings = ($env.config.keybindings | append {
      name: noren_picker
      modifier: alt_shift
      keycode: char_s
      mode: [emacs vi_normal vi_insert]
      event: { send: executehostcommand cmd: "noren" }
    })
  '';
}
```

## 3. Zellij: the switch plugin (required for inside-zellij use)

Attaching to a session from inside another session would nest clients, which
zellij forbids. noren instead pipes to the
[zellij-switch](https://github.com/mostafaqanbaryan/zellij-switch) plugin,
which must be present at `~/.config/zellij/plugins/zellij-switch.wasm`:

```nix
home.file.".config/zellij/plugins/zellij-switch.wasm".source =
  pkgs.fetchurl {
    url = "https://github.com/mostafaqanbaryan/zellij-switch/releases/download/<version>/zellij-switch.wasm";
    hash = "sha256-…";
  };
```

Outside zellij no plugin is needed (noren exec's the zellij client directly).

## 4. Zellij keybinds (config.kdl)

The recommended set — global chords for the frequent actions, plus session-mode
entries for the modal workflow:

```kdl
keybinds {
    shared_except "locked" {
        // Picker in a floating pane that closes on selection.
        bind "Alt Shift S" { Run "noren" { floating true; close_on_exit true; }; }
        // Pin toggle for the current session (it's a toggle: press again to unpin).
        bind "Alt Shift P" { Run "noren" "pin" { floating true; close_on_exit true; }; }
        // Discard: switch back to the previous session, then soft-kill this one.
        bind "Alt Shift W" { Run "noren" "discard" { floating true; close_on_exit true; }; }
        // Alt-tab for sessions: toggle current <-> previous in this window.
        // (Needs a terminal that reports Alt+Shift+Tab distinctly, e.g. via
        // the kitty keyboard protocol — foot, kitty, wezterm, ghostty do.)
        bind "Alt Shift Tab" { Run "noren" "last" { floating true; close_on_exit true; }; }
    }
    session {
        bind "s" { Run "noren" { floating true; close_on_exit true; }; SwitchToMode "locked"; }
        bind "k" { Run "noren" "pin" { floating true; close_on_exit true; }; SwitchToMode "locked"; }
        bind "x" { Run "noren" "discard" { floating true; close_on_exit true; }; SwitchToMode "locked"; }
    }
}
```

Notes:

- `floating true; close_on_exit true` gives the sesh-like floating picker that
  vanishes after selection.
- Session resurrection previews depend on zellij's serialization — make sure
  `session_serialization true` (default) is not disabled; add
  `serialize_pane_viewport true` if you want pane text restored too.

## 5. Autostart: per-project sessions for every new terminal

To make *every* shell-spawned zellij land in the right named session with the
right layout, call noren's resolver from your autostart hook. noren exposes it
in shell-friendly form:

```sh
# zellij-autostart — run from interactive shell init when outside zellij.
if command -v noren >/dev/null 2>&1; then
  eval "$(noren resolve "$PWD")"        # sets NAME / SESSION_PATH / LAYOUT / STARTUP
  name=$(noren name-for "$PWD")
else
  name=$(basename "$PWD")               # fallback without noren
fi

if zellij list-sessions -n 2>/dev/null | grep -q "^$name "; then
  exec zellij attach "$name"
elif [ -f "$PWD/.zellij.kdl" ]; then    # a local layout always wins
  exec zellij --session "$name" --new-session-with-layout "$PWD/.zellij.kdl"
elif [ -n "${LAYOUT:-}" ] && [ -f "$HOME/.config/zellij/layouts/$LAYOUT.kdl" ]; then
  [ -n "${STARTUP:-}" ] && (sleep 1; zellij --session "$name" run -c -- sh -c "$STARTUP") &
  exec zellij --session "$name" --new-session-with-layout "$HOME/.config/zellij/layouts/$LAYOUT.kdl"
else
  exec zellij attach --create "$name"
fi
```

`noren resolve` honors the full resolver chain (`[[session]]` → live session →
`[[wildcard]]` → fallback), so wildcard rules like *"everything under
~/Projects/rust gets the ide layout and runs `hx .`"* apply to plain terminal
windows, not just picker selections. `--format json` exists for non-shell
consumers.

## 6. File locations (contract)

| Path | Owner | Notes |
| --- | --- | --- |
| `~/.config/noren/config.toml` | you | all options |
| `~/.local/state/noren/last`, `previous` | noren | rotation on every connect; powers `noren last` |
| `~/.local/state/noren/pinned-dirs` | noren | pinned folder paths, one per line |
| `~/.local/state/noren/snapshots/<session>/*.kdl` | noren | UTC-stamped layout snapshots, `snapshot_keep` newest |
| `~/.local/state/zellij/pinned` | **shared** | pinned session names, one per line, trailing newline. External tools may read/append (status-bar widgets, session reapers that spare pinned sessions) — noren never writes anything but names here |
| `~/.local/state/zellij/current-session` | shared | written on `noren rename` for status-bar consumers |
| `~/.cache/zellij/contract_version_1/session_info/` | zellij | serialized layouts; noren only reads (previews, snapshots of exited sessions) |

Legacy files from noren's predecessor (`~/.config/zjp/config.toml`,
`~/.local/state/zjp/*`) are copied to the noren locations automatically on
first run; `~/.config/zjp/config.toml` also remains readable as a fallback.

## 7. Ecosystem pattern: pin-aware session reaping

Because the pin file is shared plain text, other tools can build on it. A
pattern that pairs well with noren (run from a systemd user timer):

- kill unpinned live sessions with **no attached client** after a grace period
  (they stay resurrectable),
- delete exited sessions after a retention window,
- never touch pinned sessions.

With `auto_snapshot_pinned = true`, pinned sessions additionally get layout
snapshots on every pin/connect — recoverable via `noren restore` even past the
reaper's retention window.

## 8. Verifying an install

```sh
noren list all            # all three sources merge?
noren resolve ~/some/project      # rules apply?
noren snapshot && noren snapshots # state dir writable?
noren completions fish | head     # completions generate?
```

Inside zellij, `Alt Shift S` → pick a *different* session → the client should
switch in place (no nesting error). If you get `zellij pipe` errors, the
zellij-switch plugin is missing (§3).
