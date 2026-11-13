# uima-framework dev tasks
# usage: nix develop -c just <recipe>   (or: just <recipe> from inside the shell)

default:
    @just --list

# run the tests; nextest skips doctests, so they run separately
test:
    cargo nextest run --workspace --no-tests=pass
    cargo test --doc --workspace

# format every file (rust, toml, markdown, nix, yaml; see treefmt.nix)
fmt:
    treefmt

# check formatting without modifying files
fmt-check:
    treefmt --fail-on-change

# lint (warnings are errors)
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# full pre-commit gate: formatting + lint + tests
check: fmt-check lint test

# the CI gate, as CI runs it
ci:
    nix flake check -L
