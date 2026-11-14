# Tests for the flake functions, on a fixture service. The framework's flake
# runs them as checks on Linux, where the images build.
{ pkgs, uimaLib }:
let
  inherit (pkgs) lib;

  rev = "0123456789abcdef0123456789abcdef01234567";
  sha = "0123456789ab";

  serviceWith =
    args:
    uimaLib.mkRustService (
      {
        self = {
          inherit rev;
          outPath = ./tests/fixture;
        };
        inherit (pkgs.stdenv.hostPlatform) system;
        name = "fixture";
        root = ./tests/fixture;
        mainProgram = "fixture";
        images.fixture = {
          ports = [ 8080 ];
          env.FOO = "bar";
          contents = pkgs: [ pkgs.hello ];
        };
        # The trailing slash is dropped.
        registry = "registry.test/owner/";
        source = "https://example.test/fixture";
      }
      // args
    );
  service = serviceWith { };

  # The parts evaluate as the docs say.
  failures = lib.runTests {
    testEachSystem = {
      expr =
        lib.attrNames
          (uimaLib.eachSystem (system: {
            x = system;
          })).x;
      expected = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
    };
    testPackages = {
      expr = lib.attrNames service.packages;
      expected = [
        "fixture"
        "fixture-image"
      ];
    };
    testChecks = {
      expr = lib.attrNames service.checks;
      expected = [
        "clippy"
        "fixture"
        "formatting"
      ];
    };
    testApps = {
      expr = lib.attrNames service.apps;
      expected = [
        "publish-image"
        "tag-image"
      ];
    };
    testNoRegistryNoApps = {
      expr = (serviceWith { registry = null; }).apps;
      expected = { };
    };
    testBuildSha = {
      expr = service.packages.fixture.BUILD_SHA;
      expected = sha;
    };
    testDirtyBuildSha = {
      expr = (serviceWith { self.dirtyRev = "${rev}-dirty"; }).packages.fixture.BUILD_SHA;
      expected = "${sha}-dirty";
    };
    testNoRevBuildSha = {
      expr = (serviceWith { self = { }; }).packages.fixture.BUILD_SHA;
      expected = "dev";
    };
    testMainProgram = {
      expr = lib.getExe service.packages.fixture;
      expected = "${service.packages.fixture}/bin/fixture";
    };
    testAppsHaveDescriptions = {
      expr = lib.mapAttrs (_: app: app.meta ? description) service.apps;
      expected = {
        publish-image = true;
        tag-image = true;
      };
    };
  };

  # Stand-ins for skopeo and git that log their arguments. The registry is
  # a file of `<ref> <digest>` lines.
  stubSkopeo = pkgs.writeShellScriptBin "skopeo" ''
    [[ $1 == --insecure-policy ]] && shift
    echo "$*" >> "$STUB_LOG"
    case $1 in
      copy) cat > /dev/null ;;
      inspect)
        ref=''${*: -1}
        line=$(grep "^''${ref#docker://} " "$STUB_REGISTRY") || exit 1
        echo "''${line#* }"
        ;;
    esac
  '';
  stubGit = pkgs.writeShellScriptBin "git" ''
    printf '%s\trefs/heads/main\n' "$STUB_TIP"
  '';
  scriptFor =
    file: buildSha:
    lib.getExe (
      lib.callPackageWith
        (
          pkgs
          // {
            skopeo = stubSkopeo;
            git = stubGit;
          }
        )
        file
        {
          inherit buildSha;
          registry = "registry.test/owner";
          images = {
            a = pkgs.writeShellScript "stream-a" "echo image-a";
            b = pkgs.writeShellScript "stream-b" "echo image-b";
          };
        }
    );
in
{
  test-lib =
    if failures == [ ] then
      pkgs.runCommand "test-lib" { } "touch $out"
    else
      throw "flake function tests failed: ${builtins.toJSON failures}";

  # The image has the config, users and files the docs list.
  test-image =
    pkgs.runCommand "test-image"
      {
        nativeBuildInputs = [ pkgs.jq ];
      }
      ''
        mkdir img
        ${service.packages.fixture-image} | tar -xC img
        manifest=img/manifest.json
        config=img/$(jq -r '.[0].Config' $manifest)
        jq -e '.[0].RepoTags == ["fixture:${sha}"]' $manifest
        jq -e '.config.User == "65532:65532"' $config
        jq -e '.config.Entrypoint == ["${service.packages.fixture}/bin/fixture"]' $config
        jq -e '.config.ExposedPorts == {"8080/tcp": {}}' $config
        jq -e '.config.Env | index("FOO=bar") and index("RUST_LOG=info")' $config
        jq -e '.config.Env | any(startswith("SSL_CERT_FILE=${pkgs.cacert}/"))' $config
        jq -e '.config.Labels == {
          "org.opencontainers.image.revision": "${sha}",
          "org.opencontainers.image.source": "https://example.test/fixture"
        }' $config

        # The last layer holds the files outside the store.
        root=img/$(jq -r '.[0].Layers[-1]' $manifest)
        tar -tvf $root > files
        grep -q '^drwxrwxrwt .* \./tmp/$' files
        grep -q '^l.* \./etc/ssl/certs/ca-bundle\.crt -> ${pkgs.cacert}/' files
        grep -q '^l.* \./bin/hello -> ${pkgs.hello}/' files
        [[ $(tar -xOf $root ./etc/passwd) == 'app:x:65532:65532::/tmp:/sbin/nologin' ]]

        # The app pushes to the registry without its trailing slash.
        grep -q 'docker://registry.test/owner/fixture:' ${service.apps.publish-image.program}
        touch $out
      '';

  # The apps push where the docs say, and refuse what they must.
  test-apps =
    pkgs.runCommand "test-apps"
      {
        STUB_LOG = "log";
        STUB_REGISTRY = "registry";
      }
      ''
        publish=${scriptFor ./publish-image.nix sha}
        publish_dirty=${scriptFor ./publish-image.nix "${sha}-dirty"}
        tag=${scriptFor ./tag-image.nix sha}
        tag_dirty=${scriptFor ./tag-image.nix "${sha}-dirty"}
        reg=registry.test/owner

        fail() { echo "FAIL: $*"; cat "$STUB_LOG"; exit 1; }
        # expect <status> <command...>: runs it with an empty log.
        expect() {
          local want=$1 got=0
          shift
          : > "$STUB_LOG"
          "$@" 2>/dev/null || got=$?
          [[ $got == "$want" ]] || fail "$* exited $got, want $want"
        }
        logged() { grep -qxF "$1" "$STUB_LOG" || fail "not run: $1"; }
        no_copy() { ! grep -q '^copy' "$STUB_LOG" || fail "copied"; }

        expect 0 $publish
        logged "copy docker-archive:/dev/stdin docker://$reg/a:${sha}"
        logged "copy docker-archive:/dev/stdin docker://$reg/b:${sha}"

        export STUB_TIP=${rev}
        expect 0 $publish latest
        logged "copy docker://$reg/a:${sha} docker://$reg/a:latest"
        logged "copy docker://$reg/b:${sha} docker://$reg/b:latest"

        export STUB_TIP=ffffffffffffffffffffffffffffffffffffffff
        expect 0 $publish latest
        ! grep -q ':latest' "$STUB_LOG" || fail "moved latest after main moved on"

        expect 1 $publish a/b
        no_copy
        expect 1 $publish_dirty
        no_copy

        printf '%s\n' "$reg/a:${sha} sha256:a" "$reg/b:${sha} sha256:b" > "$STUB_REGISTRY"
        expect 0 $tag v1.2.3
        logged "copy docker://$reg/a:${sha} docker://$reg/a:1.2.3"
        logged "copy docker://$reg/b:${sha} docker://$reg/b:1.2.3"

        expect 1 $tag foo
        no_copy
        expect 1 $tag 1.2
        no_copy
        expect 1 $tag_dirty 1.2.3
        no_copy

        # A re-run for the same images is fine; another image is refused.
        echo "$reg/a:1.2.3 sha256:a" >> "$STUB_REGISTRY"
        expect 0 $tag 1.2.3
        echo "$reg/b:1.2.3 sha256:other" >> "$STUB_REGISTRY"
        expect 1 $tag 1.2.3
        no_copy

        # A missing build is refused before anything is tagged.
        echo "$reg/a:${sha} sha256:a" > "$STUB_REGISTRY"
        expect 1 $tag 1.2.4
        no_copy

        touch $out
      '';
}
