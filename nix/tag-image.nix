# Tags the images main built for this commit with the release version given
# as $1, without the `v`.
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
  text = ''
    # skopeo checks images against a signature policy; there is none to
    # check here, as the images come from this build.
    skopeo() { command skopeo --insecure-policy "$@"; }

    version=''${1:?usage: tag-image <version>}
    version=''${version#v}
    sha=${buildSha}

    ${lib.concatMapStrings (binary: ''
      skopeo copy "docker://${registry}/${binary}:$sha" "docker://${registry}/${binary}:$version"
    '') (lib.attrNames images)}
  '';
}
