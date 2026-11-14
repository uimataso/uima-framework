# AGENTS.md

Guidelines for agents (and humans) working in this repo.

uima-framework holds the shared Rust crates, style guide, CI and nix setup for my services:
sowah, nova-el, techo and genkan.

- How to write the code: `docs/style-guide/`.
- How crates are laid out, named and split into light and heavy: `docs/crates.md`.
- What a project's repo has around its code and how to set it up:
  `docs/project-setup.md`.
- The plan and its decisions: `docs/plan.md`, a local file that isn't committed; read it, when
  present, before adding a crate.

## Rules

- **Rewrite, don't copy.** The services and any other existing code are reference implementations
  only. Never copy files from them; reuse their test cases as the spec for the rewrite.
- **Common things only.** No app-specific error variants, table names, routes or metric prefixes;
  take them as parameters or leave them in the app.
- **Extract what is already duplicated.** A crate needs two services that will use it.

## Checks

Run `just check test` before a commit. What each recipe runs, and what CI runs, is in
[Recipes](docs/project-setup.md#recipes).
