# Publishes each image under the build sha, and under the moving tag given as
# $1 while this commit is still main's tip. `images` maps each binary to the
# script that streams its image.
{
  lib,
  writeShellApplication,
  skopeo,
  gzip,
  git,
  buildSha,
  registry,
  images,
}:
writeShellApplication {
  name = "publish-image";
  runtimeInputs = [
    skopeo
    gzip
    git
  ];
  meta.description = "Publish the images under the build sha and a moving tag";
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images come from this build.
    skopeo() { command skopeo --insecure-policy "$@"; }

    # The moving tag can't look like a build sha or a release, the tags
    # this script and tag-image keep on one image.
    moving_tag=''${1:-}
    if [[ -n $moving_tag ]] && { ! [[ $moving_tag =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]] ||
      [[ $moving_tag =~ ^[0-9a-f]{12}$ ]] || [[ $moving_tag =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }; then
      echo "publish-image: not a moving tag: $moving_tag" >&2
      exit 1
    fi
    sha=${buildSha}
    if ! [[ $sha =~ ^[0-9a-f]{12}$ ]]; then
      echo "publish-image: not a clean commit build: $sha" >&2
      exit 1
    fi

    ${lib.concatStrings (
      lib.mapAttrsToList (binary: image: ''
        # skopeo reads a compressed archive from a pipe; an uncompressed one
        # needs a file it can seek in.
        ${image} | gzip --fast |
          skopeo copy docker-archive:/dev/stdin "docker://${registry}/${binary}:$sha"
      '') images
    )}
    if [[ -n $moving_tag ]]; then
      tip=$(git ls-remote origin refs/heads/main | cut -c1-12)
      if [[ -z $tip ]]; then
        echo "publish-image: origin has no main; $moving_tag stays" >&2
        exit 1
      fi
      if [[ $tip != "$sha" ]]; then
        echo "publish-image: main has moved on to $tip; $moving_tag stays" >&2
        exit 0
      fi
      ${lib.concatMapStrings (binary: ''
        skopeo copy "docker://${registry}/${binary}:$sha" \
          "docker://${registry}/${binary}:$moving_tag"
      '') (lib.attrNames images)}
    fi
  '';
}
