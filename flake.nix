{
  description = "uima-framework: the common base for my services";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      rust-overlay,
      treefmt-nix,
    }:
    let
      inherit (nixpkgs) lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forAllSystems = lib.genAttrs systems;

      # The toolchain rust-toolchain.toml names, for both the dev shell and
      # the checks. A release newer than the locked rust-overlay needs
      # `nix flake update rust-overlay` first.
      toolchainFor =
        system:
        (rust-overlay.lib.mkRustBin { } nixpkgs.legacyPackages.${system}).fromRustupToolchainFile
          ./rust-toolchain.toml;

      # treefmt with every formatter in treefmt.nix.
      treefmtFor =
        system:
        treefmt-nix.lib.evalModule nixpkgs.legacyPackages.${system} (
          import ./treefmt.nix { rustfmt = toolchainFor system; }
        );
    in
    {
      # `nix fmt`
      formatter = forAllSystems (system: (treefmtFor system).config.build.wrapper);

      # The static half of the CI gate, the same formatting and lints as
      # `just check`. Tests run outside the sandbox, through `just test`.
      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchainFor system);
          common = {
            pname = "uima-framework";
            version = "0.0.0";
            # Only the Rust sources and Cargo files, so a docs change doesn't
            # rebuild anything.
            src = craneLib.cleanCargoSource ./.;
          };
          # The dependencies, built once and cached apart from the crates.
          cargoArtifacts = craneLib.buildDepsOnly common;
        in
        {
          # Every file is formatted as treefmt.nix says.
          formatting = (treefmtFor system).config.build.check self;

          # A library workspace has nothing to install, so the check is
          # clippy alone.
          clippy = craneLib.cargoClippy (
            common
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets --all-features -- -D warnings";
            }
          );
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          toolchain = toolchainFor system;
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              toolchain

              (treefmtFor system).config.build.wrapper
              cargo-nextest
              cargo-mutants
              just
              git
            ];

            # lets rust-analyzer find std sources
            RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
          };
        }
      );
    };
}
