# The flake functions a project's flake.nix builds on. What they set up, and
# why, is in docs/project-setup.md.
{
  nixpkgs,
  crane,
  rust-overlay,
  treefmt-nix,
}:
let
  inherit (nixpkgs) lib;

  # The commit's first 12 hex digits, with `-dirty` for uncommitted changes.
  buildShaOf =
    self:
    if self ? rev then
      builtins.substring 0 12 self.rev
    else if self ? dirtyRev then
      builtins.substring 0 12 self.dirtyRev + "-dirty"
    else
      "dev";

  systems = [
    "x86_64-linux"
    "aarch64-linux"
    "aarch64-darwin"
    "x86_64-darwin"
  ];

  # The parts every Rust project has: the toolchain rust-toolchain.toml
  # names, the formatting and clippy checks, the formatter and the dev
  # shell.
  #
  # `extraSrc` lists the files the build reads besides the Rust sources and
  # Cargo files, such as an `include_str!` asset. `devPackages` are added to
  # the dev shell.
  mkRustProject =
    {
      self,
      system,
      name,
      root,
      extraSrc ? [ ],
      devPackages ? [ ],
    }:
    let
      pkgs = nixpkgs.legacyPackages.${system};

      # A release newer than the locked rust-overlay needs
      # `nix flake update rust-overlay` in the framework first.
      toolchain = (rust-overlay.lib.mkRustBin { } pkgs).fromRustupToolchainFile (
        root + "/rust-toolchain.toml"
      );

      treefmt = treefmt-nix.lib.evalModule pkgs {
        imports = [
          ./cargo-sort.nix
          (root + "/treefmt.nix")
        ];
        programs.rustfmt.package = toolchain;
      };

      craneLib = (crane.mkLib pkgs).overrideToolchain (_: toolchain);

      common = {
        pname = name;
        inherit (craneLib.crateNameFromCargoToml { cargoToml = root + "/Cargo.toml"; }) version;
        src = lib.fileset.toSource {
          inherit root;
          fileset = lib.fileset.unions ([ (craneLib.fileset.commonCargoSources root) ] ++ extraSrc);
        };
        strictDeps = true;
      };

      # The dependencies, built once and reused by the build and the checks.
      cargoArtifacts = craneLib.buildDepsOnly common;
    in
    {
      inherit
        pkgs
        toolchain
        craneLib
        common
        cargoArtifacts
        ;

      formatter = treefmt.config.build.wrapper;

      checks = {
        formatting = treefmt.config.build.check self;

        clippy = craneLib.cargoClippy (
          common
          // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--workspace --all-targets --all-features -- -D warnings";
          }
        );
      };

      devShell = pkgs.mkShell {
        packages = [
          toolchain
          treefmt.config.build.wrapper
          pkgs.cargo-nextest
          pkgs.just
          pkgs.git
        ]
        ++ devPackages;

        # lets rust-analyzer find std sources
        RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
      };
    };

  # Turns `f system` into flake outputs: `{ packages = ...; }` for each
  # system becomes `{ packages.<system> = ...; }`.
  eachSystem =
    f:
    lib.foldl' lib.recursiveUpdate { } (
      map (system: lib.mapAttrs (_: value: { ${system} = value; }) (f system)) systems
    );

  # A service: `mkRustProject`, plus the package with every binary, an image
  # per binary, and the apps that publish them.
  #
  # `images.<binary>` takes `ports`, `env`, `contents` and `uid`, all
  # optional. `registry` is where the images go, such as
  # `ghcr.io/<owner>`; `source` is the repo URL for the image label.
  mkRustService =
    {
      images ? { },
      registry ? null,
      source ? null,
      mainProgram ? null,
      ...
    }@args:
    let
      project = mkRustProject (
        removeAttrs args [
          "images"
          "registry"
          "source"
          "mainProgram"
        ]
      );
      inherit (project)
        pkgs
        craneLib
        common
        cargoArtifacts
        ;
      inherit (args) name;
      buildSha = buildShaOf args.self;

      package = craneLib.buildPackage (
        common
        // {
          inherit cargoArtifacts;
          # The tests run outside the sandbox, through `just test`.
          doCheck = false;
          # Set here rather than in `common`, so a new commit doesn't
          # rebuild the dependencies.
          BUILD_SHA = buildSha;
          meta = lib.optionalAttrs (mainProgram != null) { inherit mainProgram; };
        }
      );

      imageOf =
        binary:
        {
          ports ? [ ],
          env ? { },
          contents ? [ ],
          uid ? 65532,
        }:
        pkgs.dockerTools.streamLayeredImage {
          name = binary;
          tag = buildSha;
          contents = [ pkgs.cacert ] ++ contents;
          config = {
            User = "${toString uid}:${toString uid}";
            Entrypoint = [ "${package}/bin/${binary}" ];
            Env = lib.mapAttrsToList (key: value: "${key}=${value}") (
              {
                RUST_LOG = "info";
                SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
              }
              // env
            );
            ExposedPorts = lib.genAttrs (map (port: "${toString port}/tcp") ports) (_: { });
            Labels = {
              "org.opencontainers.image.revision" = buildSha;
            }
            // lib.optionalAttrs (source != null) { "org.opencontainers.image.source" = source; };
          };
        };

      imagePackages = lib.mapAttrs' (
        binary: config: lib.nameValuePair "${binary}-image" (imageOf binary config)
      ) images;

      # An app from nix/<name>.nix, for the images this build makes.
      imageApp = name: {
        type = "app";
        program = lib.getExe (
          pkgs.callPackage ./${name}.nix {
            inherit buildSha registry;
            images = lib.mapAttrs (binary: _: imagePackages."${binary}-image") images;
          }
        );
      };

      # dockerTools images are Linux-only.
      isLinux = pkgs.stdenv.hostPlatform.isLinux;
    in
    {
      inherit (project) formatter devShell;

      packages = {
        ${name} = package;
      }
      // lib.optionalAttrs isLinux imagePackages;

      checks = project.checks // {
        ${name} = package;
      };

      apps = lib.optionalAttrs (isLinux && registry != null && images != { }) {
        publish-image = imageApp "publish-image";
        tag-image = imageApp "tag-image";
      };
    };
in
{
  inherit
    systems
    eachSystem
    mkRustProject
    mkRustService
    ;
}
