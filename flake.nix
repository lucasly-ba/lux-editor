{
  description = "Development environment for lux, a modal terminal text editor.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      # Systems this flake supports. lux is developed on x86_64 Linux; add
      # more here if it ever needs to build elsewhere.
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      # The editor itself, as an installable package. Build with `nix build`,
      # install with `nix profile install`, or run straight from the repo with
      # `nix run`.
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "lux";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          # makeWrapper to put rust-analyzer on the runtime PATH; pkg-config in
          # case a -sys crate probes for libraries. The C toolchain comes from
          # the default stdenv (rustc shells out to `cc`, and tree-sitter's
          # grammars are C compiled at build time).
          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          # rust-analyzer is an optional *runtime* dependency: lux spawns it for
          # diagnostics and completion. Wrapping it onto PATH means LSP keeps
          # working when `lux` is launched globally, outside the dev shell.
          postInstall = ''
            wrapProgram $out/bin/lux \
              --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.rust-analyzer ]}
          '';
          meta = {
            description = "A modal, Helix-inspired terminal text editor written from scratch in Rust.";
            mainProgram = "lux";
          };
        };
      });

      # `nix run` launches the editor (e.g. `nix run . -- samples/demo.rs`).
      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = pkgs.lib.getExe self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        };
      });

      # `nix develop` (and `use flake` via direnv) drop you into this shell.
      # Once inside, use `cargo build` / `cargo test` / `cargo run` as usual.
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            # tree-sitter language grammars are C and get compiled by the `cc`
            # crate at build time, and rustc shells out to `cc` to link the
            # final binary. Without a C toolchain the build fails with
            # `linker `cc` not found`.
            gcc
            # Some -sys crates probe the system with pkg-config.
            pkg-config
          ];
        };
      });
    };
}
