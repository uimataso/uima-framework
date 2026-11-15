# Tags the images published under a build sha with each tag given. With
# `--no-overwrite`, a tag already on another image is refused, as a release
# is. `binaries` names the images.
{
  lib,
  writeShellApplication,
  skopeo,
  registry,
  binaries,
}:
writeShellApplication {
  name = "tag-image";
  runtimeInputs = [ skopeo ];
  meta.description = "Tag the images published under a build sha";
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images are this project's own.
    skopeo() { command skopeo --insecure-policy "$@"; }
    # Prints the image's digest; exits 2 when the registry has no such
    # image, and 1 on any other error, such as auth or the network.
    digest() {
      local err status=0
      err=$(skopeo inspect --format '{{.Digest}}' "docker://$1" 2>&1 >&3) || status=$?
      if ((status == 0)); then
        return 0
      elif ((status == 2)) || [[ $err == *"manifest unknown"* ]]; then
        return 2
      fi
      echo "$err" >&2
      return 1
    } 3>&1

    no_overwrite=false
    if [[ ''${1:-} == --no-overwrite ]]; then
      no_overwrite=true
      shift
    fi
    if (($# < 2)); then
      echo "usage: tag-image [--no-overwrite] <sha> <tag>..." >&2
      exit 1
    fi
    sha=$1
    shift
    if ! [[ $sha =~ ^[0-9a-f]{12}$ ]]; then
      echo "tag-image: not a build sha: $sha" >&2
      exit 1
    fi
    for tag in "$@"; do
      if ! [[ $tag =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$ ]]; then
        echo "tag-image: not a tag: $tag" >&2
        exit 1
      fi
    done

    registry=${lib.escapeShellArg registry}
    binaries=(${lib.escapeShellArgs binaries})

    # Check every image first, so a refusal leaves no tag half moved.
    for binary in "''${binaries[@]}"; do
      image=$registry/$binary
      status=0
      built=$(digest "$image:$sha") || status=$?
      if ((status == 2)); then
        echo "tag-image: $image:$sha is missing" >&2
        exit 1
      elif ((status != 0)); then
        exit 1
      fi
      if $no_overwrite; then
        for tag in "$@"; do
          status=0
          tagged=$(digest "$image:$tag") || status=$?
          if ((status == 0)) && [[ $tagged != "$built" ]]; then
            echo "tag-image: $image:$tag is already another image" >&2
            exit 1
          elif ((status == 1)); then
            exit 1
          fi
        done
      fi
    done

    for binary in "''${binaries[@]}"; do
      for tag in "$@"; do
        skopeo copy "docker://$registry/$binary:$sha" "docker://$registry/$binary:$tag"
      done
    done
  '';
}
