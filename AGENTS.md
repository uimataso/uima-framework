# AGENTS.md

Guidelines for agents (and humans) working in this repo.

uima-framework is the framework my services are built on, so a service writes only its business
code and its vendor integrations. It holds the Rust crates, style guide, CI and nix setup for
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
- **Own everything but the business.** The framework holds what a service needs that isn't its
  business: plumbing, wiring and infrastructure, even when only one service has it today. Domain
  rules, use cases, routes and vendor integrations stay in the service.

## Checks

Before a commit, run the checks [Recipes](docs/project-setup.md#recipes) names; it also says what
each recipe and CI run.
