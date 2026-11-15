# Prints the version a release tag names: `v1.2.3` or `refs/tags/v1.2.3`
# gives `1.2.3`.
{ writeShellApplication }:
writeShellApplication {
  name = "release-version";
  meta.description = "Print the version a release tag names";
  text = ''
    if (($# != 1)); then
      echo "usage: release-version <tag>" >&2
      exit 1
    fi
    ref=$1
    if ! [[ ''${ref#refs/tags/} =~ ^v([0-9]+\.[0-9]+\.[0-9]+)$ ]]; then
      echo "release-version: not a release tag: $ref" >&2
      exit 1
    fi
    echo "''${BASH_REMATCH[1]}"
  '';
}
