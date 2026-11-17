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
      nativeLibs ? _: [ ],
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
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = nativeLibs pkgs;
      };

      # The dependencies, built once and reused by the build and the checks.
      # The check covers what clippy checks, so clippy rebuilds none of them.
      # No `cargo test --no-run`: the tests run outside the sandbox, through
      # `just test`.
      cargoArtifacts = craneLib.buildDepsOnly (
        common
        // {
          cargoCheckExtraArgs = "--workspace --all-targets --all-features";
          doCheck = false;
        }
      );

      # The node flake.lock locks the `uima` input at, and the framework's
      # owner and rev when it is locked from github; a local override has no
      # rev to pin the actions to.
      lock = root + "/flake.lock";
      frameworkNode =
        let
          nodes = (lib.importJSON lock).nodes;
          input = nodes.root.inputs.uima or null;
        in
        if !lib.pathExists lock then
          null
        else if lib.isList input then
          # `follows` another input's.
          { locked = { }; }
        else if input != null then
          nodes.${input}
        else
          null;
      framework =
        let
          inherit (frameworkNode.locked) owner rev;
        in
        if frameworkNode != null && frameworkNode.locked.type or null == "github" then
          # Both go into the script's patterns.
          assert lib.assertMsg (
            builtins.match "[A-Za-z0-9-]+" owner != null
          ) "flake.lock: uima's owner isn't one: ${owner}";
          assert lib.assertMsg (
            builtins.match "[0-9a-f]{40}" rev != null
          ) "flake.lock: uima's rev isn't one: ${rev}";
          {
            inherit owner rev;
          }
        else
          null;
      syncActions = pkgs.callPackage ./sync-actions.nix { inherit (framework) owner rev; };
      workflows = lib.fileset.toSource {
        inherit root;
        fileset = lib.fileset.unions [
          (lib.fileset.maybeMissing (root + "/.github/workflows"))
          (lib.fileset.maybeMissing (root + "/.forgejo/workflows"))
        ];
      };
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

      apps = lib.optionalAttrs (framework != null) {
        sync-actions = {
          type = "app";
          program = lib.getExe syncActions;
          inherit (syncActions) meta;
        };
      };

      checks =
        lib.warnIf (frameworkNode != null && framework == null)
          "flake.lock doesn't lock uima from github itself, so the actions check is skipped"
          (
            lib.optionalAttrs (framework != null) {
              # The workflows use the framework's actions at the rev the
              # flake locks.
              actions = pkgs.runCommand "actions" { } ''
                cd ${workflows}
                ${lib.getExe syncActions} --check
                touch $out
              '';
            }
          )
        // {
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
          # Ahead of any pkg-config on the host, which would link the host's
          # libraries.
          pkgs.pkg-config
        ]
        ++ devPackages pkgs;
        buildInputs = nativeLibs pkgs;

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
  # `nativeLibs` is `pkgs: [ ... ]`, the C libraries a crate finds through
  # pkg-config, for the build and the dev shell.
  mkRustProject =
    args:
    let
      parts = projectParts args;
    in
    {
      inherit (parts)
        formatter
        apps
        checks
        devShell
        ;
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

      # The script from nix/<app>.nix, called with `args`.
      scriptOf = app: args: pkgs.callPackage ./${app}.nix args;
      # That script as an app.
      appOf =
        app: args:
        let
          script = scriptOf app args;
        in
        {
          type = "app";
          program = lib.getExe script;
          inherit (script) meta;
        };

      tagImageArgs = {
        registry = registry';
        binaries = lib.attrNames images;
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

      apps =
        project.apps
        // lib.optionalAttrs (isLinux && registry != null && images != { }) {
          publish-image = appOf "publish-image" {
            inherit buildSha;
            registry = registry';
            images = lib.mapAttrs (binary: _: imagePackages."${binary}-image") images;
          };
          tag-image = appOf "tag-image" tagImageArgs;
          build-sha = appOf "build-sha" { inherit buildSha; };
          release-version = appOf "release-version" { };
          is-tip = appOf "is-tip" { };
          move-tag = appOf "move-tag" {
            inherit buildSha;
            isTip = scriptOf "is-tip" { };
            tagImage = scriptOf "tag-image" tagImageArgs;
          };
          wait-for-build = appOf "wait-for-build" {
            inherit buildSha;
            isTip = scriptOf "is-tip" { };
          };
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
