{
  description = "Monitor Claude Code sessions in tmux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
      in {
        packages = rec {
          clux = pkgs.rustPlatform.buildRustPackage {
            pname = "clux";
            inherit version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
          };

          tmuxPlugin = pkgs.tmuxPlugins.mkTmuxPlugin {
            pluginName = "clux";
            inherit version;
            src = self;
            postInstall = ''
              mkdir -p $out/share/tmux-plugins/clux/bin
              cp ${clux}/bin/clux $out/share/tmux-plugins/clux/bin/clux
              echo "${version}" > $out/share/tmux-plugins/clux/bin/.version
            '';
          };

          default = clux;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [ cargo rustc clippy rustfmt ];
        };
      });
}
