# Tags the images main built for this commit with the release version given
# as $1, without the `v`. `images` maps each binary to its image; only the
# names are used.
{
  lib,
  writeShellApplication,
  skopeo,
  buildSha,
  registry,
  images,
}:
writeShellApplication {
  name = "tag-image";
  runtimeInputs = [ skopeo ];
  meta.description = "Tag the images main built for this commit with a release version";
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images come from this build.
    skopeo() { command skopeo --insecure-policy "$@"; }
    digest() { skopeo inspect --format '{{.Digest}}' "docker://$1"; }

    version=''${1:?usage: tag-image <version>}
    version=''${version#v}
    if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      echo "tag-image: not a release version: $1" >&2
      exit 1
    fi
    sha=${buildSha}
    if ! [[ $sha =~ ^[0-9a-f]{12}$ ]]; then
      echo "tag-image: not a clean commit build: $sha" >&2
      exit 1
    fi

    # Check every image first, so a refusal leaves no tag half moved: each
    # build must exist, and a release already tagged stays on its image.
    ${lib.concatMapStrings (binary: ''
      if ! built=$(digest "${registry}/${binary}:$sha"); then
        echo "tag-image: ${registry}/${binary}:$sha is missing; wait for main's run" >&2
        exit 1
      fi
      if released=$(digest "${registry}/${binary}:$version" 2>/dev/null) &&
        [[ $released != "$built" ]]; then
        echo "tag-image: ${registry}/${binary}:$version is already another image" >&2
        exit 1
      fi
    '') (lib.attrNames images)}
    ${lib.concatMapStrings (binary: ''
      skopeo copy "docker://${registry}/${binary}:$sha" \
        "docker://${registry}/${binary}:$version"
    '') (lib.attrNames images)}
  '';
}
