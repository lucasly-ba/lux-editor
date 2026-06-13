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
