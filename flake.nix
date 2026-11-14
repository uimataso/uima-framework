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
          devPackages = [ nixpkgs.legacyPackages.${system}.cargo-mutants ];
        };
      in
      {
        inherit (project) formatter checks;
        devShells.default = project.devShell;
      }
    );
}
