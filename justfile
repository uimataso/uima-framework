# uima-framework dev tasks
# usage: nix develop -c just <recipe>   (or: just <recipe> from inside the shell)

default:
    @just --list

# run the tests; nextest skips doctests, so they run separately
test: test-nix
    cargo nextest run --workspace --no-tests=pass
    cargo test --doc --workspace

# the flake checks named `test-*`, such as the flake functions' tests
test-nix:
    #!/usr/bin/env bash
    set -euo pipefail
    checks=".#checks.$(nix eval --impure --raw --expr builtins.currentSystem)"
    nix eval --raw "$checks" --apply 'cs: toString (builtins.filter
      (n: builtins.match "test-.*" n != null) (builtins.attrNames cs))' \
      | tr ' ' '\n' | sed "s|^|$checks.|" | xargs --no-run-if-empty nix build --no-link

# format every file (rust, toml, markdown, nix, yaml; see treefmt.nix)
fmt:
    treefmt

# check formatting without modifying files
fmt-check:
    treefmt --fail-on-change

# lint (warnings are errors)
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# static checks: formatting + lint; `just check test` before a commit
check: fmt-check lint

# mutation testing: break the code in small ways and check a test fails;
# slower than `test`, so it isn't part of `check` or CI
mutants *args:
    cargo mutants --workspace --test-tool nextest {{args}}

# the CI gate, which CI runs: the static checks and build in nix, then the tests
ci:
    nix flake check -L
    NEXTEST_PROFILE=ci nix develop -c just test
