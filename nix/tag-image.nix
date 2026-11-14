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
      status=0
      built=$(digest "${registry}/${binary}:$sha") || status=$?
      if ((status == 2)); then
        echo "tag-image: ${registry}/${binary}:$sha is missing; wait for main's run" >&2
        exit 1
      elif ((status != 0)); then
        exit 1
      fi
      status=0
      released=$(digest "${registry}/${binary}:$version") || status=$?
      if ((status == 0)) && [[ $released != "$built" ]]; then
        echo "tag-image: ${registry}/${binary}:$version is already another image" >&2
        exit 1
      elif ((status == 1)); then
        exit 1
      fi
    '') (lib.attrNames images)}
    ${lib.concatMapStrings (binary: ''
      skopeo copy "docker://${registry}/${binary}:$sha" \
        "docker://${registry}/${binary}:$version"
    '') (lib.attrNames images)}
  '';
}
