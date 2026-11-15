# Moves each tag to this build's images while the build is still the
# branch's tip on origin, and leaves them when the branch has moved on, for
# a newer build's run to move.
{
  lib,
  writeShellApplication,
  buildSha,
  isTip,
  tagImage,
}:
writeShellApplication {
  name = "move-tag";
  meta.description = "Move tags to this build while it is the branch's tip";
  # The checks' ranges, such as [0-9a-f], match ASCII only.
  runtimeEnv.LC_ALL = "C";
  text = ''
    if (($# < 2)); then
      echo "usage: move-tag <branch> <tag>..." >&2
      exit 1
    fi
    branch=$1
    shift
    # Checked here too, so a bad tag fails on every run, not only at the tip.
    for tag in "$@"; do
      if ! [[ $tag =~ ${tagImage.tagPattern} ]]; then
        echo "move-tag: not a tag: $tag" >&2
        exit 1
      fi
    done
    sha=${lib.escapeShellArg buildSha}
    tip=$(${lib.getExe isTip} "$branch" "$sha")
    if [[ $tip != true ]]; then
      echo "move-tag: $branch has moved on from $sha; a newer build's run moves $*"
      exit 0
    fi
    ${lib.getExe tagImage} "$sha" "$@"
  '';
}
