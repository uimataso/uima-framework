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
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images come from this build.
    skopeo() { command skopeo --insecure-policy "$@"; }

    moving_tag=''${1:-}
    sha=${buildSha}
    if ! [[ $sha =~ ^[0-9a-f]{12}$ ]]; then
      echo "publish-image: not a clean commit build: $sha" >&2
      exit 1
    fi

    ${lib.concatStrings (
      lib.mapAttrsToList (binary: image: ''
        ${image} | gzip --fast |
          skopeo copy docker-archive:/dev/stdin "docker://${registry}/${binary}:$sha"
      '') images
    )}
    if [[ -n $moving_tag ]]; then
      tip=$(git ls-remote origin refs/heads/main | cut -c1-12)
      if [[ $tip != "$sha" ]]; then
        echo "publish-image: main has moved on to $tip; $moving_tag stays" >&2
        exit 0
      fi
      ${lib.concatMapStrings (binary: ''
        skopeo copy "docker://${registry}/${binary}:$sha" "docker://${registry}/${binary}:$moving_tag"
      '') (lib.attrNames images)}
    fi
  '';
}
