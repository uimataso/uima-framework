# Pins every `<owner>/uima-framework/actions/<name>@<ref>` in the workflows
# under .github/workflows and .forgejo/workflows to `rev`, the framework's
# rev in flake.lock, with or without a `https://github.com/` in front. With
# --check it changes nothing, lists each framework action at another ref,
# owner, host or case, and fails when there are any.
{
  writeShellApplication,
  gnugrep,
  gnused,
  owner,
  rev,
}:
writeShellApplication {
  name = "sync-actions";
  runtimeInputs = [
    gnugrep
    gnused
  ];
  meta.description = "Pin the workflows' framework actions to flake.lock's rev";
  # The patterns' ranges, such as [0-9a-f], match ASCII only.
  runtimeEnv.LC_ALL = "C";
  text = ''
    check=false
    case $* in
      "") ;;
      --check) check=true ;;
      *)
        echo "usage: sync-actions [--check]" >&2
        exit 1
        ;;
    esac
    files=()
    for dir in .github/workflows .forgejo/workflows; do
      # Neither mode follows a link, which may leave the repo or lead
      # nowhere.
      if [[ -L $dir ]]; then
        echo "sync-actions: workflows behind a link aren't supported: $dir" >&2
        exit 1
      fi
      if [[ -d $dir ]]; then
        if links=$(find "$dir" -type l -print) && [[ -n $links ]]; then
          echo "sync-actions: workflows behind a link aren't supported: $links" >&2
          exit 1
        fi
        while IFS= read -r -d "" file; do
          files+=("$file")
        done < <(find "$dir" -type f \( -name '*.yaml' -o -name '*.yml' \) -print0 | sort -z)
      fi
    done
    action='(https://github\.com/)?${owner}/uima-framework/actions/[A-Za-z0-9_-]+@'
    ref='[A-Za-z0-9._/-]+'
    if $check; then
      stale=false
      for file in "''${files[@]}"; do
        # Each word that names a framework action, in any case, on its own:
        # it must be the framework's, at the rev.
        # grep fails with 1 on no match, and with 2 on a file it can't read.
        words=$(grep -ioE "[^[:space:]\"'{},]*uima-framework/actions/[^[:space:]\"'{},]*" "$file") || {
          status=$?
          ((status == 1)) || exit 2
          continue
        }
        while IFS= read -r word; do
          if ! [[ $word =~ ^''${action}${rev}$ ]]; then
            echo "$file: $word"
            stale=true
          fi
        done <<<"$words"
      done
      if $stale; then
        echo "sync-actions: these aren't uima-framework's actions at flake.lock's ${rev}; run \`nix run .#sync-actions\`, which leaves another owner, host or case for you to fix" >&2
        exit 1
      fi
    elif ((''${#files[@]} > 0)); then
      sed -i -E \
        "s#(^|[[:space:]\"'{,])($action)$ref#\\1\\2${rev}#g" "''${files[@]}"
    fi
  '';
}
