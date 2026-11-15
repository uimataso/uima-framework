# Publishes each image under the build sha. `images` maps each binary to the
# script that streams its image.
{
  lib,
  writeShellApplication,
  skopeo,
  gzip,
  buildSha,
  registry,
  images,
}:
writeShellApplication {
  name = "publish-image";
  runtimeInputs = [
    skopeo
    gzip
  ];
  meta.description = "Publish this build's images under its sha";
  # The checks' ranges, such as [0-9a-f], match ASCII only.
  runtimeEnv.LC_ALL = "C";
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images come from this build.
    skopeo() { command skopeo --insecure-policy "$@"; }

    if (($#)); then
      echo "usage: publish-image" >&2
      exit 1
    fi
    sha=${buildSha}
    if ! [[ $sha =~ ^[0-9a-f]{12}$ ]]; then
      echo "publish-image: not a clean commit build: $sha" >&2
      exit 1
    fi

    registry=${lib.escapeShellArg registry}
    ${lib.concatStrings (
      lib.mapAttrsToList (binary: image: ''
        # skopeo reads a compressed archive from a pipe; an uncompressed one
        # needs a file it can seek in.
        ${image} | gzip --fast |
          skopeo copy docker-archive:/dev/stdin "docker://$registry/${binary}:$sha"
      '') images
    )}
  '';
}
