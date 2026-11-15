# Exits 0 when the sha is the tip of the branch on origin, 1 when it isn't,
# and 2 when the tip can't be read.
{
  writeShellApplication,
  git,
}:
writeShellApplication {
  name = "is-tip";
  runtimeInputs = [ git ];
  meta.description = "Check a sha is the tip of a branch on origin";
  text = ''
    if (($# != 2)); then
      echo "usage: is-tip <branch> <sha>" >&2
      exit 2
    fi
    branch=$1
    sha=$2
    if ! [[ $sha =~ ^[0-9a-f]{7,40}$ ]]; then
      echo "is-tip: not a sha: $sha" >&2
      exit 2
    fi
    if ! refs=$(git ls-remote origin "refs/heads/$branch"); then
      exit 2
    fi
    # ls-remote matches the pattern's tail, so keep the exact ref.
    tip=
    while read -r hash ref; do
      if [[ $ref == "refs/heads/$branch" ]]; then
        tip=$hash
      fi
    done <<< "$refs"
    if [[ -z $tip ]]; then
      echo "is-tip: origin has no $branch" >&2
      exit 2
    fi
    [[ $tip == "$sha"* ]]
  '';
}
