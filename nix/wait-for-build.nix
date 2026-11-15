# Waits until the URL, a deploy's healthcheck, answers with this build's
# sha as the JSON field `sha`, and fails after the timeout. A build that is
# no longer the branch's tip on origin has nothing to wait for: a newer
# build's run waits for its own.
{
  lib,
  writeShellApplication,
  curl,
  jq,
  buildSha,
  isTip,
}:
writeShellApplication {
  name = "wait-for-build";
  runtimeInputs = [
    curl
    jq
  ];
  meta.description = "Wait until a deploy serves this build while it is the branch's tip";
  # The checks' ranges, such as [0-9], match ASCII only.
  runtimeEnv.LC_ALL = "C";
  text = ''
    usage() {
      echo "usage: wait-for-build [--timeout <seconds>] <branch> <url>" >&2
      exit 1
    }
    timeout=300
    if [[ ''${1:-} == --timeout ]]; then
      (($# >= 2)) || usage
      timeout=$2
      shift 2
    fi
    (($# == 2)) || usage
    if ! [[ $timeout =~ ^[0-9]{1,6}$ ]]; then
      echo "wait-for-build: not a number of seconds: $timeout" >&2
      exit 1
    fi
    branch=$1
    url=$2
    sha=${lib.escapeShellArg buildSha}
    # moved_on: exits when the branch has moved on from this build.
    moved_on() {
      local tip
      # Its own line, so a failing is-tip ends the run.
      tip=$(${lib.getExe isTip} "$branch" "$sha")
      if [[ $tip != true ]]; then
        echo "wait-for-build: $branch has moved on from $sha; a newer build's run waits"
        exit 0
      fi
    }
    moved_on
    end=$((SECONDS + 10#$timeout))
    while true; do
      # The poll and the sleep end by the timeout, or a second after it,
      # curl's least --max-time.
      left=$((end - SECONDS))
      served=$(curl -sSfL --max-time $((left < 1 ? 1 : left > 10 ? 10 : left)) -- "$url" | jq -r '.sha // empty') ||
        served=
      if [[ $served == "$sha" ]]; then
        echo "wait-for-build: $url serves $sha"
        exit 0
      fi
      left=$((end - SECONDS))
      if ((left <= 0)); then
        echo "wait-for-build: $url still serves ''${served:-nothing} after ''${timeout}s, not $sha" >&2
        # The deploy may have taken a newer build instead.
        moved_on
        exit 1
      fi
      sleep $((left < 10 ? left : 10))
    done
  '';
}
