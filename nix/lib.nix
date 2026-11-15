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
  ];

  # Everything `mkRustProject` sets up, including what `mkRustService`
  # builds on.
  projectParts =
    {
      self,
      system,
      name,
      root,
      extraSrc ? [ ],
      devPackages ? _: [ ],
    }:
    let
      pkgs = nixpkgs.legacyPackages.${system};

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
      # The check covers what clippy checks, so clippy rebuilds none of them.
      cargoArtifacts = craneLib.buildDepsOnly (
        common // { cargoCheckExtraArgs = "--workspace --all-targets --all-features"; }
      );
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
        ++ devPackages pkgs;

        # lets rust-analyzer find std sources
        RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
      };
    };

  # The parts every Rust project has: the formatting and clippy checks, the
  # formatter and the dev shell, all on the toolchain rust-toolchain.toml
  # names.
  #
  # `extraSrc` lists the files the build reads besides the Rust sources and
  # Cargo files, such as an `include_str!` asset or a test fixture.
  # `devPackages` is `pkgs: [ ... ]`, the tools added to the dev shell.
  mkRustProject =
    args:
    let
      parts = projectParts args;
    in
    {
      inherit (parts) formatter checks devShell;
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
  # `images.<binary>` takes `ports` (TCP), `env`, `contents` (`pkgs: [ ... ]`)
  # and `uid`, all optional. `registry` is where the images go, such as
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
      project = projectParts (
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
      registry' = lib.removeSuffix "/" registry;

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
          contents ? _: [ ],
          uid ? 65532,
        }:
        (pkgs.dockerTools.streamLayeredImage {
          name = binary;
          tag = buildSha;
          contents = [ pkgs.cacert ] ++ contents pkgs;
          # A writable /tmp, and a user for the uid, so temp files and $HOME
          # work.
          extraCommands = ''
            mkdir -m 1777 tmp
            mkdir -p etc
            echo "app:x:${toString uid}:${toString uid}::/tmp:/sbin/nologin" > etc/passwd
            echo "app:x:${toString uid}:" > etc/group
          '';
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
        }).overrideAttrs
          (old: {
            # An `images` key that isn't a binary fails here, not when the
            # container starts.
            buildCommand = ''
              if ! test -x ${package}/bin/${binary}; then
                echo "no binary ${binary} in ${package}" >&2
                exit 1
              fi
            ''
            + old.buildCommand;
          });

      imagePackages = lib.mapAttrs' (
        binary: config: lib.nameValuePair "${binary}-image" (imageOf binary config)
      ) images;

      # An app from nix/<app>.nix, for the images this build makes.
      imageApp =
        app:
        let
          script = pkgs.callPackage ./${app}.nix {
            inherit buildSha;
            registry = registry';
            images = lib.mapAttrs (binary: _: imagePackages."${binary}-image") images;
          };
        in
        {
          type = "app";
          program = lib.getExe script;
          inherit (script) meta;
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
    eachSystem
    mkRustProject
    mkRustService
    ;
}
