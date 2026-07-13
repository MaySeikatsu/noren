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
  pname = "zjp3";
  version = "0.1.0";

  src = lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [ makeWrapper installShellFiles ];

  # Completions come from the binary itself (`zjp3 completions <shell>`);
  # fish/zsh/bash land in the standard vendor dirs, nushell has no standard
  # location so it goes to share/nushell/ and is sourced by zjp.nix.
  postInstall = ''
    installShellCompletion --cmd zjp3 \
      --fish <($out/bin/zjp3 completions fish) \
      --zsh  <($out/bin/zjp3 completions zsh) \
      --bash <($out/bin/zjp3 completions bash)
    mkdir -p $out/share/nushell
    $out/bin/zjp3 completions nushell > $out/share/nushell/zjp3-completions.nu
  '';

  # Runtime deps must be on PATH — zjp3 shells out to zellij / zoxide / fzf /
  # git / eza (preview fallback). zellij-switch.wasm is installed by
  # zellij.nix and found via $HOME, so no wrapping needed for it.
  postFixup = ''
    wrapProgram $out/bin/zjp3 \
      --prefix PATH : ${lib.makeBinPath [ zellij zoxide fzf git eza coreutils ]}
  '';

  meta = with lib; {
    description = "Sesh-parity zellij session picker (Rust)";
    homepage = "local";
    license = licenses.mit;
    platforms = platforms.linux;
    mainProgram = "zjp3";
  };
}
