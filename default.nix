{ lib
, rustPlatform
, makeWrapper
, installShellFiles
, zellij
, zoxide
, fzf
, git
, eza
, coreutils
}:

rustPlatform.buildRustPackage {
  pname = "noren";
  version = "0.1.0";

  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ makeWrapper installShellFiles ];

  # Completions come from the binary itself (`noren completions <shell>`);
  # fish/zsh/bash land in the standard vendor dirs, nushell has no standard
  # location so it goes to share/nushell/ and is sourced by zjp.nix.
  postInstall = ''
    installShellCompletion --cmd noren \
      --fish <($out/bin/noren completions fish) \
      --zsh  <($out/bin/noren completions zsh) \
      --bash <($out/bin/noren completions bash)
    mkdir -p $out/share/nushell
    $out/bin/noren completions nushell > $out/share/nushell/noren-completions.nu
  '';

  # Runtime deps must be on PATH — noren shells out to zellij / zoxide / fzf /
  # git / eza (preview fallback). zellij-switch.wasm is installed by
  # zellij.nix and found via $HOME, so no wrapping needed for it.
  postFixup = ''
    wrapProgram $out/bin/noren \
      --prefix PATH : ${lib.makeBinPath [ zellij zoxide fzf git eza coreutils ]}
  '';

  meta = with lib; {
    description = "Sesh-parity zellij session picker (Rust)";
    homepage = "local";
    license = licenses.mit;
    platforms = platforms.linux;
    mainProgram = "noren";
  };
}
