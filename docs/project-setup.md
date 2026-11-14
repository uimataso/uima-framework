# Project setup

What a uima project's repo has around its code, and how to set it up. This repo follows it, so its
own files are the working example.

## Recipes

Every project's `justfile` has these recipes, and adds its own (such as `run`) beside them:

| Recipe      | Runs                                                                             |
| ----------- | -------------------------------------------------------------------------------- |
| `fmt`       | Formats every file.                                                              |
| `fmt-check` | Checks the formatting without changing files.                                    |
| `lint`      | The linters, warnings as errors.                                                 |
| `check`     | `fmt-check` and `lint`.                                                          |
| `test`      | Every test the project has, Rust or not.                                         |
| `ci`        | The gate as CI runs it: `nix flake check`, then `test` with the CI test profile. |

`nix flake check` runs the same checks as `check`; the tests stay out of it (see [Tests](#tests)).
Before a commit, run `just check test`.

## Toolchain

- **`rust-toolchain.toml`** at the repo root is the only place the Rust version is written. Copy
  this repo's [`rust-toolchain.toml`](../rust-toolchain.toml), components included. It names an
  exact release, not `stable`, so a new release's lints can't break a build that passed.
- **`flake.nix`** reads it with [rust-overlay] for the build, the checks and the dev shell, so CI
  gets the same release. See `toolchainFor` in this repo's [`flake.nix`](../flake.nix). A release
  newer than the locked rust-overlay needs `nix flake update rust-overlay` first.
- **Bumping**: each project bumps on its own schedule, within what
  [depending on the framework](crates.md#depending-on-the-framework) allows.
- **`rust-version`**: a library sets it to the oldest Rust it builds with. A service leaves it out;
  its `rust-toolchain.toml` already says which Rust it uses.

## Build

`flake.nix` builds with [crane]. See the `clippy` check in this repo's [`flake.nix`](../flake.nix).

- **Dependencies** build once in their own derivation and are reused by the build and the checks,
  so a change to the project's code doesn't rebuild them. Git dependencies need no hash.
- **Sources**: `cleanCargoSource`, which keeps the Rust sources and Cargo files. Add any other file
  the build reads, such as `migrations/` for `sqlx::migrate!` or an asset under `include_str!`.
- **Packages** are named, with no `packages.default`: `packages.<project>` holds every binary of
  the workspace, built once. A single-binary project sets `meta.mainProgram`, so
  `nix run .#<project>` runs it.
- **Build sha**: the build sets `BUILD_SHA` to the commit, `<sha>-dirty` with uncommitted changes.
  The app's `main` reads it with `option_env!("BUILD_SHA")`, falling back to `"dev"` in a plain
  `cargo build`, and hands it to whatever reports it. A framework crate doesn't read it itself,
  since its value would be fixed when the dependencies are built.
- **TLS** is rustls, not OpenSSL, so neither the build nor the image needs a system library.
- **Tests** don't run in the build; see [Tests](#tests).
- **Profile**: Cargo's default release profile.

## Dev shell

`nix develop` gives the toolchain and every tool the recipes run. A project adds the tools its own
recipes and tests need beside them.

## Formatting

Every file in the repo is formatted, not only Rust, with [treefmt] through [treefmt-nix].

- **`treefmt.nix`** at the repo root lists the formatters. Start from this repo's
  [`treefmt.nix`](../treefmt.nix) and add a formatter for each other kind of file the project has.
- **rustfmt** comes from the toolchain `rust-toolchain.toml` pins, so the formatting doesn't change
  with whichever rustfmt is on `PATH`. It runs with its defaults: no `rustfmt.toml`.
- **`flake.nix`** turns `treefmt.nix` into the `formatter` (`nix fmt`), a `formatting` check, and a
  `treefmt` in the dev shell. See `treefmtFor` and its uses in this repo's
  [`flake.nix`](../flake.nix).

When adding formatting to an existing repo, format everything in a commit of its own and list it
in `.git-blame-ignore-revs`. GitHub and Forgejo read the file on their own; locally, run
`git config blame.ignoreRevsFile .git-blame-ignore-revs` once per clone.

```sh
# .git-blame-ignore-revs
# style: format every file with treefmt
<full commit sha>
```

## Lints

rustc's and Clippy's default lints, plus a few more that the project's `Cargo.toml` turns on.

- **The lint table**: copy this repo's `[workspace.lints]` from [`Cargo.toml`](../Cargo.toml).
  Cargo can't share lints across repos, so each project keeps its own copy. Keep the lints at
  `warn`, not `deny`, so code in progress still builds.
- **Workspaces**: the table goes in `[workspace.lints]`, and every crate opts in with
  `[lints] workspace = true`; without that, the crate gets none of them. A single-crate project
  puts the table in `[lints]`.
- **Coverage**: Clippy checks every target and every feature, so the tests and the code behind the
  `testing` feature are linted too.
- **Silencing a lint**: prefer `#[expect(lint, reason = "...")]` to `#[allow(...)]`. Once the
  lint no longer fires, `expect` warns, so the attribute doesn't outlive its reason.

## Tests

- **Runner**: the tests run with [nextest], and the doctests with `cargo test --doc`, since nextest
  can't run doctests yet ([nextest#16]). Once it can, nextest runs both.
- **Outside the sandbox**: the tests run in the dev shell, not in `nix flake check`, so they can
  reach Docker and the project's other tools.
- **CI profile**: copy this repo's [`.config/nextest.toml`](../.config/nextest.toml). CI runs
  nextest with its `ci` profile, which reports every failure and kills a hung test.
- **External services**: a test that needs a service, such as Postgres, starts it with
  [testcontainers]; see the [style guide](style-guide/testing.md#test-harness).
- **Proptest**: commit the `proptest-regressions/` files, so a case that once failed runs every
  time.

[crane]: https://crane.dev
[nextest]: https://nexte.st
[nextest#16]: https://github.com/nextest-rs/nextest/issues/16
[testcontainers]: https://github.com/testcontainers/testcontainers-rs
[rust-overlay]: https://github.com/oxalica/rust-overlay
[treefmt]: https://github.com/numtide/treefmt
[treefmt-nix]: https://github.com/numtide/treefmt-nix
