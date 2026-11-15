# Prints `true` when the sha, 12 to 40 hex digits, is the tip of the branch
# on origin, and `false` when it isn't; fails when the tip can't be read, so
# a workflow that sets -e stops there.
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
      exit 1
    fi
    branch=$1
    sha=$2
    if ! [[ $sha =~ ^[0-9a-f]{12,40}$ ]]; then
      echo "is-tip: not a sha: $sha" >&2
      exit 1
    fi
    if ! refs=$(git ls-remote origin "refs/heads/$branch"); then
      exit 1
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
      exit 1
    fi
    if [[ $tip == "$sha"* ]]; then
      echo true
    else
      echo false
    fi
  '';
}
