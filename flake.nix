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
      uimaLib = import ./nix/lib.nix {
        inherit
          nixpkgs
          crane
          rust-overlay
          treefmt-nix
          ;
      };
    in
    {
      lib = uimaLib;
    }
    // uimaLib.eachSystem (
      system:
      let
        # A library workspace has nothing to install, so it is a project,
        # not a service.
        project = uimaLib.mkRustProject {
          inherit self system;
          name = "uima-framework";
          root = ./.;
          # `sqlx::migrate!` reads these at compile time.
          extraSrc = [
            ./crates/lock/postgres-sqlx/migrations
            ./crates/lock/postgres-sqlx/testdata
          ];
          devPackages = pkgs: [ pkgs.cargo-mutants ];
        };
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        inherit (project) formatter;
        checks =
          project.checks
          # The tests build images, which are Linux-only.
          // nixpkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
            import ./nix/tests.nix { inherit pkgs uimaLib; }
          );
        devShells.default = project.devShell;
      }
    );
}
