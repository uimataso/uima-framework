# AGENTS.md

Guidelines for agents (and humans) working in this repo.

uima-framework holds the shared Rust crates, style guide, CI and nix setup for the uima services:
sowah, nova-el, techo and genkan.

- How to write the code: `docs/style-guide/`.
- How crates are laid out, named and split into light and heavy: `docs/crates.md`.
- The plan and its decisions: `docs/plan.md`, a local file that isn't committed; read it, when
  present, before adding a crate.

## Rules

- **Rewrite, don't copy.** The services and any other existing code are reference implementations
  only. Never copy files from them; reuse their test cases as the spec for the rewrite.
- **Common things only.** No app-specific error variants, table names, routes or metric prefixes;
  take them as parameters or leave them in the app.
- **Extract what is already duplicated.** A crate needs two services that will use it.

## Checks

`just check` runs fmt, clippy (warnings are errors) and nextest. CI runs the same through
`nix flake check` (`just ci`).
