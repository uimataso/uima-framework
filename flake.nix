{
  description = "uima-framework — the common base for uima's services";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
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
      fenix,
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
      # the checks. Bump `sha256` with the channel: set it to lib.fakeHash,
      # and nix prints the right one.
      toolchainFor =
        system:
        fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-MFHoWVcPZKjFrLnQnodCXbUsfZcofGfo9tV9fTTvklg=";
        };

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

      # The CI gate: the same fmt + clippy + test as `just check`.
      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          toolchain = toolchainFor system;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        {
          # Every file is formatted as treefmt.nix says.
          formatting = (treefmtFor system).config.build.check self;

          # A library workspace has nothing to install, so this is one
          # derivation that runs clippy and the tests.
          workspace = pkgs.stdenv.mkDerivation {
            name = "uima-framework-check";

            # Only what cargo needs.
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./crates
              ];
            };

            cargoDeps = rustPlatform.importCargoLock { lockFile = ./Cargo.lock; };

            nativeBuildInputs = with pkgs; [
              rustPlatform.cargoSetupHook
              toolchain
              cargo-nextest
            ];

            buildPhase = ''
              runHook preBuild
              cargo clippy --workspace --all-targets --offline -- -D warnings
              cargo nextest run --workspace --offline --no-tests=pass
              runHook postBuild
            '';

            installPhase = ''touch "$out"'';
          };
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
