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
        extraSrc = [ ./tests/fixture/asset.txt ];
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
        "build-sha"
        "is-tip"
        "publish-image"
        "release-version"
        "tag-image"
      ];
    };
    testAppPrograms = {
      expr = lib.filterAttrs (name: app: baseNameOf app.program != name) service.apps;
      expected = { };
    };
    testNoRegistryNoApps = {
      expr = (serviceWith { registry = null; }).apps;
      expected = { };
    };
    # dockerTools images are Linux-only.
    testNoLinuxNoApps = {
      expr = (serviceWith { system = "aarch64-darwin"; }).apps;
      expected = { };
    };
    testNoImagesNoApps = {
      expr = (serviceWith { images = { }; }).apps;
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
      expr = service.packages.fixture.meta.mainProgram or null;
      expected = "fixture";
    };
    testAppsHaveDescriptions = {
      expr = lib.filterAttrs (_: app: !(app.meta ? description)) service.apps;
      expected = { };
    };
  };

  # Stand-ins for skopeo and git that log their arguments, and fail on
  # arguments the apps must not pass, noting why in $STUB_BAD, since an app
  # may turn the failure into its own. The registry is a file of
  # `<ref> <digest>` lines; a ref in $STUB_BROKEN fails as an unreachable
  # registry would. A missing ref fails as skopeo does, with exit 2, or
  # with $STUB_MISSING_STATUS.
  stubSkopeo = pkgs.writeShellScriptBin "skopeo" ''
    if [[ $1 != --insecure-policy ]]; then
      echo "skopeo without --insecure-policy" >> "$STUB_BAD"
      exit 3
    fi
    shift
    echo "$*" >> "$STUB_LOG"
    ref=''${*: -1}
    ref=''${ref#docker://}
    if [[ $ref == "''${STUB_BROKEN:-}" ]]; then
      echo "unauthorized: authentication required" >&2
      exit 1
    fi
    case $1 in
      copy)
        if [[ $2 == docker-archive:/dev/stdin ]] && ! gzip -t; then
          echo "the archive isn't gzip" >> "$STUB_BAD"
          exit 3
        fi
        ;;
      inspect)
        if ! line=$(grep "^$ref " "$STUB_REGISTRY"); then
          echo "manifest unknown: manifest unknown" >&2
          exit "''${STUB_MISSING_STATUS:-2}"
        fi
        echo "''${line#* }"
        ;;
    esac
  '';
  # git's origin has main at $STUB_TIP, or no main when it's empty, and
  # always a branch whose name ends in main; $STUB_GIT_FAIL fails it.
  stubGit = pkgs.writeShellScriptBin "git" ''
    if [[ "$*" != "ls-remote origin refs/heads/main" ]]; then
      echo "unexpected git $*" >> "$STUB_BAD"
      exit 3
    fi
    if [[ -n ''${STUB_GIT_FAIL:-} ]]; then
      echo "fatal: could not read from remote repository" >&2
      exit 128
    fi
    printf '%s\trefs/heads/old/main\n' ffffffffffffffffffffffffffffffffffffffff
    if [[ -n ''${STUB_TIP:-} ]]; then
      printf '%s\trefs/heads/main\n' "$STUB_TIP"
    fi
  '';
  # The build of `drv` fails, with `text` in its log.
  failsWith =
    text: drv:
    pkgs.runCommand "fails-with" { } ''
      grep -qF ${lib.escapeShellArg text} ${pkgs.testers.testBuildFailure drv}/testBuildFailure.log
      touch $out
    '';

  # The app from `file`, called with `args`, on the stubs.
  appFor =
    file: args:
    lib.getExe (
      lib.callPackageWith (
        pkgs
        // {
          skopeo = stubSkopeo;
          git = stubGit;
        }
      ) file args
    );
  publishFor =
    buildSha:
    appFor ./publish-image.nix {
      inherit buildSha;
      registry = "registry.test/owner";
      images = {
        a = pkgs.writeShellScript "stream-a" "echo image-a";
        b = pkgs.writeShellScript "stream-b" "echo image-b";
      };
    };
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
        jq -e '.config.Env | index("SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt")' $config
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
        [[ $(tar -xOf $root ./etc/group) == 'app:x:65532:' ]]

        # The apps push this build, to the registry without its trailing
        # slash.
        publish=${service.apps.publish-image.program}
        grep -q '^sha=${sha}$' $publish
        grep -q '/fixture:$sha"' $publish
        for app in $publish ${service.apps.tag-image.program}; do
          grep -qE "^registry='?registry\.test/owner'?$" $app
        done
        grep -qE "^binaries=\('?fixture'?\)$" ${service.apps.tag-image.program}
        [[ $(${service.apps.build-sha.program}) == ${sha} ]]
        touch $out
      '';

  # The apps push where the docs say, and refuse what they must.
  test-apps =
    pkgs.runCommand "test-apps"
      {
        STUB_LOG = "log";
        STUB_BAD = "bad";
        STUB_REGISTRY = "registry";
      }
      ''
        publish=${publishFor sha}
        publish_dirty=${publishFor "${sha}-dirty"}
        tag=${
          appFor ./tag-image.nix {
            registry = "registry.test/owner";
            binaries = [
              "a"
              "b"
            ];
          }
        }
        publish_odd=${
          appFor ./publish-image.nix {
            buildSha = sha;
            registry = "registry.test/o;wner";
            images.a = pkgs.writeShellScript "stream-a" "echo image-a";
          }
        }
        tag_odd=${
          appFor ./tag-image.nix {
            registry = "registry.test/o;wner";
            binaries = [ "a" ];
          }
        }
        version=${appFor ./release-version.nix { }}
        is_tip=${appFor ./is-tip.nix { }}
        reg=registry.test/owner

        fail() { echo "FAIL: $*"; cat "$STUB_LOG"; exit 1; }
        # expect <status> <command...>: runs it with an empty log.
        expect() {
          local want=$1 got=0
          shift
          : > "$STUB_LOG"
          : > "$STUB_BAD"
          "$@" > out 2> err || got=$?
          [[ ! -s $STUB_BAD ]] || fail "$*: $(cat "$STUB_BAD")"
          [[ $got == "$want" ]] || fail "$* exited $got, want $want"
        }
        logged() { grep -qxF "$1" "$STUB_LOG" || fail "not run: $1"; }
        no_copy() { ! grep -q '^copy' "$STUB_LOG" || fail "copied"; }

        # publish-image pushes this build under its sha, and nothing else.
        expect 0 $publish
        logged "copy docker-archive:/dev/stdin docker://$reg/a:${sha}"
        logged "copy docker-archive:/dev/stdin docker://$reg/b:${sha}"
        [[ $(grep -c . "$STUB_LOG") == 2 ]] || fail "pushed more than the sha"
        expect 1 $publish latest
        no_copy
        expect 1 $publish_dirty
        no_copy
        STUB_BROKEN=$reg/a:${sha} expect 1 $publish

        # tag-image copies the sha to each tag, on every image.
        printf '%s\n' "$reg/a:${sha} sha256:a" "$reg/b:${sha} sha256:b" > "$STUB_REGISTRY"
        long=$(printf 'a%.0s' {1..128})
        expect 0 $tag ${sha} latest "$long" x Latest
        [[ $(grep -c '^copy' "$STUB_LOG") == 8 ]] || fail "copied more than the tags"
        for image in a b; do
          logged "copy docker://$reg/$image:${sha} docker://$reg/$image:latest"
          logged "copy docker://$reg/$image:${sha} docker://$reg/$image:x"
          logged "copy docker://$reg/$image:${sha} docker://$reg/$image:Latest"
          logged "copy docker://$reg/$image:${sha} docker://$reg/$image:$long"
        done

        # Not a sha, not a tag, or nothing to tag. The registry has the
        # refs, so only the check refuses them.
        for image in a b; do
          printf "%s\n" "$reg/$image:${sha}-dirty sha256:$image" "$reg/$image:${rev} sha256:$image" >> "$STUB_REGISTRY"
        done
        for bad in ${sha}-dirty 0123 ${rev}; do
          expect 1 $tag "$bad" latest
          no_copy
        done
        for bad in a/b .x -x "a$long"; do
          expect 1 $tag ${sha} latest "$bad"
          no_copy
        done
        expect 1 $tag ${sha}
        expect 1 $tag --no-overwrite ${sha}

        # Without --no-overwrite a tag moves; with it, a tag on another image
        # is refused, and one on the same image is a re-run.
        echo "$reg/b:1.2.3 sha256:other" >> "$STUB_REGISTRY"
        expect 0 $tag ${sha} 1.2.3
        logged "copy docker://$reg/b:${sha} docker://$reg/b:1.2.3"
        expect 1 $tag --no-overwrite ${sha} 1.2.4 1.2.3
        no_copy
        echo "$reg/a:1.2.5 sha256:a" >> "$STUB_REGISTRY"
        expect 0 $tag --no-overwrite ${sha} 1.2.5
        logged "copy docker://$reg/b:${sha} docker://$reg/b:1.2.5"

        # A tag or a build that can't be looked up is refused.
        STUB_BROKEN=$reg/b:1.2.6 expect 1 $tag --no-overwrite ${sha} 1.2.6
        no_copy
        # Only skopeo's status says a tag is missing, not the message.
        STUB_MISSING_STATUS=1 expect 1 $tag --no-overwrite ${sha} 1.2.6
        no_copy
        STUB_BROKEN=$reg/b:${sha} expect 1 $tag ${sha} latest
        no_copy
        grep -q unauthorized err || fail "the registry's error isn't shown"

        # A missing build is refused before anything is tagged.
        echo "$reg/a:${sha} sha256:a" > "$STUB_REGISTRY"
        expect 1 $tag ${sha} latest
        no_copy
        grep -qF "$reg/b:${sha} is missing" err || fail "the missing build isn't named"

        # The registry reaches skopeo as written, even with shell characters.
        odd='registry.test/o;wner'
        echo "$odd/a:${sha} sha256:a" > "$STUB_REGISTRY"
        expect 0 $publish_odd
        logged "copy docker-archive:/dev/stdin docker://$odd/a:${sha}"
        expect 0 $tag_odd ${sha} latest
        logged "copy docker://$odd/a:${sha} docker://$odd/a:latest"

        # release-version reads a release tag's version.
        for ref in v1.2.3 refs/tags/v1.2.3; do
          expect 0 $version "$ref"
          [[ $(cat out) == 1.2.3 ]] || fail "$ref gave $(cat out)"
        done
        for bad in 1.2.3 v1.2 v1.2.3-rc1 v1.2.3rc v1x2x3 refs/heads/v1.2.3 xv1.2.3 ""; do
          expect 1 $version "$bad"
        done
        expect 1 $version
        expect 1 $version v1.2.3 v1.2.4

        # is-tip: true at main's tip, false elsewhere, and fails when it
        # can't tell.
        tip() { expect 0 $is_tip main "$1"; [[ $(cat out) == "$2" ]] || fail "$1 gave $(cat out)"; }
        export STUB_TIP=${rev}
        tip ${sha} true
        tip ${rev} true
        tip ffffffffffff false
        tip 3456789abcde false
        tip 0123456789abffffffffffffffffffffffffffff false
        for bad in ${sha}-dirty 0123456789a ${rev}0 0123456789AB; do
          expect 1 $is_tip main "$bad"
        done
        expect 1 $is_tip main
        expect 1 $is_tip main ${sha} extra
        STUB_GIT_FAIL=1 expect 1 $is_tip main ${sha}
        STUB_TIP= expect 1 $is_tip main ffffffffffff

        touch $out
      '';

  # An `images` key that isn't a binary fails the image build.
  test-missing-binary =
    failsWith "no binary nope"
      (serviceWith {
        images.nope = { };
      }).packages.nope-image;

  # The cargo-sort program sorts, nested Cargo.toml files too: an unsorted
  # member fails the check.
  test-cargo-sort =
    failsWith "member/Cargo.toml"
      (uimaLib.mkRustProject {
        self.outPath = ./tests/unsorted;
        inherit (pkgs.stdenv.hostPlatform) system;
        name = "unsorted";
        root = ./tests/unsorted;
      }).checks.formatting;

  # cargo-sort and taplo leave a sorted Cargo.toml as it is: groups stay
  # apart, and arrays keep taplo's indent.
  test-cargo-sort-sorted =
    (uimaLib.mkRustProject {
      self.outPath = ./tests/sorted;
      inherit (pkgs.stdenv.hostPlatform) system;
      name = "sorted";
      root = ./tests/sorted;
    }).checks.formatting;

  # Clippy denies warnings, in the tests too: a crate with one fails the
  # check.
  test-clippy =
    failsWith "len_zero"
      (uimaLib.mkRustProject {
        self.outPath = ./tests/warning;
        inherit (pkgs.stdenv.hostPlatform) system;
        name = "warning";
        root = ./tests/warning;
      }).checks.clippy;
}
