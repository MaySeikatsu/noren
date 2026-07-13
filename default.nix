{ lib
, rustPlatform
, makeWrapper
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

  nativeBuildInputs = [ makeWrapper ];

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
