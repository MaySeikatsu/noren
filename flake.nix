{
  description = "zjp3 - sesh-parity zellij session manager (Rust)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: let
    systems = [ "x86_64-linux" "aarch64-linux" ];
    forAllSystems = f: nixpkgs.lib.genAttrs systems (system:
      f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: rec {
      zjp3 = pkgs.callPackage ./default.nix { };
      default = zjp3;
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          # runtime deps for manual testing
          zellij
          zoxide
          fzf
        ];
      };
    });
  };
}
