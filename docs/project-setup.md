# Project setup

What a project's repo has around its code, and how to set it up. This repo follows it, so its
own files are the working example.

## Recipes

Every project's `justfile` has these recipes, and adds its own (such as `run`) beside them:

| Recipe      | Runs                                                                                          |
| ----------- | --------------------------------------------------------------------------------------------- |
| `fmt`       | Formats every file.                                                                           |
| `fmt-check` | Checks the formatting without changing files.                                                 |
| `lint`      | The linters, warnings as errors.                                                              |
| `check`     | `fmt-check` and `lint`.                                                                       |
| `test`      | Every test the project has, Rust or not.                                                      |
| `ci`        | The whole gate: `nix flake check`, then `test` with the CI test profile. CI runs this recipe. |

`nix flake check` runs the same checks as `check`; the tests stay out of it (see [Tests](#tests)).
Before a commit, run `just check test`.

## Flake

A project's `flake.nix` calls the framework's flake functions, in [`nix/lib.nix`](../nix/lib.nix),
which list their parameters: `mkRustService` for a service, `mkRustProject` for a library. They
return one system's outputs; the project places them with `eachSystem`, so its own `flake.nix`
shows what it outputs, and an extra check or app goes beside them with `//`. This repo's
[`flake.nix`](../flake.nix) is the library example.

## Toolchain

- **`rust-toolchain.toml`** at the repo root is the only place the Rust version is written. Copy
  this repo's [`rust-toolchain.toml`](../rust-toolchain.toml), components included. It names an
  exact release, not `stable`, so a new release's lints can't break a build that passed.
- **The flake functions** read it with [rust-overlay] for the build, the checks and the dev shell,
  so CI gets the same release. A release newer than the framework's locked rust-overlay needs
  `nix flake update rust-overlay` in this repo first.
- **Bumping**: each project bumps on its own schedule, within what
  [depending on the framework](crates.md#depending-on-the-framework) allows.
- **`rust-version`**: a library sets it to the oldest Rust it builds with. A service leaves it out;
  its `rust-toolchain.toml` already says which Rust it uses.

## Build

`mkRustService` builds with [crane].

- **Dependencies** build once in their own derivation and are reused by the build and the checks,
  so a change to the project's code doesn't rebuild them. Git dependencies need no hash.
- **Sources**: `cleanCargoSource`, which keeps the Rust sources and Cargo files. Add any other file
  the build reads, such as `migrations/` for `sqlx::migrate!` or an asset under `include_str!`,
  through `extraSrc`.
- **Packages** are named, with no `packages.default`: `packages.<project>` holds every binary of
  the workspace, built once. A single-binary project sets `meta.mainProgram`, so
  `nix run .#<project>` runs it.
- **Build sha**: the build sets `BUILD_SHA` to the commit's first 12 hex digits, with `-dirty`
  added for uncommitted changes. The image tag uses the same 12 digits, so the binary, what it
  reports and the image name one commit the same way.
  The app's `main` reads it with `option_env!("BUILD_SHA")`, falling back to `"dev"` in a plain
  `cargo build`, and hands it to whatever reports it. A framework crate doesn't read it itself,
  since its value would be fixed when the dependencies are built.
- **TLS** is rustls, not OpenSSL, so neither the build nor the image needs a system library.
- **Tests** don't run in the build; see [Tests](#tests).
- **Profile**: Cargo's default release profile.

## Image

`mkRustService` builds an image per binary in `images`, `packages.<binary>-image`, with nix's
`dockerTools.streamLayeredImage`; there is no Dockerfile.

- **Contents**: the binary and the CA certificates (`SSL_CERT_FILE` points at them). A project adds
  what its binary runs, such as a CLI it calls; debugging tools stay out.
- **User**: not root. The default uid is 65532; a project picks another when its volumes need one.
- **Config**: `ExposedPorts` and `Env` come from the project, with `RUST_LOG=info` by default.
- **Labels**: `org.opencontainers.image.revision` (the build sha) and
  `org.opencontainers.image.source` (the repo URL).
- **Tags**: the build sha on every image. A release, tagged `v1.2.3` in git, also gets `1.2.3`,
  without the `v`. A moving tag such as `latest` or `dev` is the project's to choose, for its
  deploy.
- **Architecture**: the one the CI runner has.

## CI

Each project's workflow runs on its own CI host, in three stages:

1. **Every push**: `nix develop -c just ci`, so the gate is defined once, in the `justfile`.
2. **Main**: build the images and publish each under its build sha and the project's moving tag,
   such as `latest` or `dev`. The moving tag moves only while the commit is still main's tip, so a
   slow run can't move it back. Then the project's own deploy steps, if any.
3. **A release tag** (`v1.2.3`): tag the image main already built for that commit as `1.2.3`,
   without rebuilding it, so production runs the image dev ran.

- **Publishing** goes through the flake's apps, `publish-image` and `tag-image`, so CI and a local
  run push the same way. The CI
  host only logs in to the registry.
- **Actions** (GitHub) are pinned to a commit sha, with the version in a comment.
- **Triggers**: `push` and `workflow_dispatch`. Every job has a timeout.
- **Nix cache**: the store is kept between runs, by `cache-nix-action` on GitHub and by a volume on
  the Forgejo runner.
- **Cargo cache**: the tests compile outside the store, so cargo's downloads and `target/` are
  cached too. The downloads may restore from an older `Cargo.lock`; `target/` only from the same
  `Cargo.lock` and `rust-toolchain.toml`, so stale builds don't pile up.

### Image retention

Each registry's own cleanup rules keep:

- every release tag (`1.2.3`);
- the images the moving tags point at;
- the 10 most recent images that have only a build sha tag.

## Dev shell

`nix develop` gives the toolchain and every tool the recipes run. A project adds the tools its own
recipes and tests need through `devPackages`.

## Formatting

Every file in the repo is formatted, not only Rust, with [treefmt] through [treefmt-nix].

- **`treefmt.nix`** at the repo root lists the formatters. Start from this repo's
  [`treefmt.nix`](../treefmt.nix) and add a formatter for each other kind of file the project has.
- **rustfmt** comes from the toolchain `rust-toolchain.toml` pins, so the formatting doesn't change
  with whichever rustfmt is on `PATH`. It runs with its defaults: no `rustfmt.toml`.
- **The flake functions** turn `treefmt.nix` into the `formatter` (`nix fmt`), a `formatting`
  check, and a `treefmt` in the dev shell. They also add a `cargo-sort` program, which
  treefmt-nix doesn't have.

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
