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

`nix flake check` runs the same checks as `check`, and for a service also builds it; the Rust tests
stay out of it (see [Tests](#tests)).
Before a commit, run `just check test`.

## Flake

A project's `flake.nix` calls the framework's flake functions, in [`nix/lib.nix`](../nix/lib.nix),
which list their parameters: `mkRustService` for a service, `mkRustProject` for a library. They
return one system's parts, such as `checks` and `devShell`; the project places them with
`eachSystem`, so its own `flake.nix` shows what it outputs, and an extra check or app goes beside
them with `//`. This repo's [`flake.nix`](../flake.nix) is the library example.

## Toolchain

- **`rust-toolchain.toml`** at the repo root is the only place the toolchain release is written.
  Copy this repo's [`rust-toolchain.toml`](../rust-toolchain.toml), components included. It names
  an exact release, not `stable`, so a new release's lints can't break a build that passed.
- **The flake functions** read it with [rust-overlay] for the build, the checks and the dev shell,
  so CI gets the same release. A release newer than the framework's locked rust-overlay needs
  `nix flake update rust-overlay` in this repo first.
- **Bumping**: each project bumps on its own schedule, within what
  [depending on the framework](crates.md#depending-on-the-framework) allows.
- **`rust-version`**: a library sets it to the oldest Rust it builds with. A service leaves it out;
  its `rust-toolchain.toml` already says which Rust it uses.

## Build

The flake functions build with [crane].

- **Dependencies** build once in their own derivation and are reused by the build and the checks,
  so a change to the project's code doesn't rebuild them. Git dependencies need no hash.
- **Sources**: the Rust sources and Cargo files. Add any other file the build or a test reads, such
  as `migrations/` for `sqlx::migrate!` or a fixture under `include_str!`, through `extraSrc`.
- **Packages** are named, with no `packages.default`: `packages.<project>` holds every binary of
  the workspace, built once. A single-binary project passes `mainProgram`, so
  `nix run .#<project>` runs it.
- **Build sha**: the build sets `BUILD_SHA` to the commit's first 12 hex digits, with `-dirty`
  added for uncommitted changes. The image tag uses the same 12 digits, so the binary, what it
  reports and the image name one commit the same way.
  The app's `main` reads it with `option_env!("BUILD_SHA")`, falling back to `"dev"` in a plain
  `cargo build`, and hands it to whatever reports it. A framework crate doesn't read it itself,
  since its value would be fixed when the dependencies are built. A healthcheck that
  `wait-for-build` polls reports it as the JSON field `sha`.
- **TLS** is rustls, not OpenSSL, so neither the build nor the image needs a system library.
- **C libraries** a crate finds through pkg-config go in `nativeLibs`; the build, the dev shell and
  the image then use the nix one. A library not listed isn't found, so a `-sys` crate that can
  builds its bundled copy.
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
- **Tags**: see [CI](#ci).
- **Architecture**: the one the CI runner has.

## CI

Each project's workflow runs on its own CI host, in three stages:

1. **Every push**: `nix develop -c just ci`, so the gate is defined once, in the `justfile`.
2. **Main**: publish the images under the build sha, then move the project's moving tag, such as
   `latest`, only while the commit is still main's tip, so a slow run can't move it back. The tag
   stays where it is until a run of main's tip passes, so a failed tip holds it on an older commit.
   Then the project's own deploy steps, if any.
3. **A release tag** (`v1.2.3`): tag the images main already published for that commit as `1.2.3`,
   without rebuilding them, so production runs the image dev ran. A release already on another
   image is refused, and so is a pre-release tag. Push the tag once main's run has published the
   commit, and while the registry still keeps that image; a tag pushed earlier fails, and a re-run
   tags it. Only maintainers can push release tags, through the host's tag protection.

- **Publishing** goes through the flake's apps, so CI and a local run push the same way. Each app
  does one operation; which branch publishes and which tags move is the workflow's to decide:

  | App                                             | Does                                                                                                |
  | ----------------------------------------------- | --------------------------------------------------------------------------------------------------- |
  | `publish-image`                                 | Pushes each image under the build sha.                                                              |
  | `tag-image [--no-overwrite] <sha> <tag>...`     | Tags each image's build sha; `--no-overwrite` refuses a tag on another image.                       |
  | `build-sha`                                     | Prints the build sha.                                                                               |
  | `release-version <tag>`                         | Prints the version a release tag names: `v1.2.3` gives `1.2.3`.                                     |
  | `is-tip <branch> <sha>`                         | Prints `true` at the branch's tip on `origin`, `false` elsewhere.                                   |
  | `move-tag <branch> <tag>...`                    | Tags the build sha while it is the branch's tip, and leaves the tags if not.                        |
  | `wait-for-build [--timeout <s>] <branch> <url>` | Waits until `<url>` serves the build sha while it is the branch's tip, and fails after the timeout. |

- **Forgejo jobs** run in a `nixos/nix` container, which has no node for actions/checkout; this
  repo's [`actions/`](../actions) check out and log in to the registry with shell steps instead.
- **Framework actions** are pinned to the rev `flake.lock` locks the framework at; the `actions`
  check fails on any other. After `nix flake update uima`, `nix run .#sync-actions` pins them
  again.
- **Publish jobs** run only for main and release tags. They log in to the registry and have an
  `origin` they can read.
- **Registry credentials**: on GitHub, an environment that only main and release tags may deploy
  to holds them. Forgejo can't scope a secret to refs, so anyone who can push can read it; only
  maintainers get push access there.
- **One at a time**: the jobs that tag images, with `move-tag` or the release's `tag-image`, share
  one `concurrency` group, so they run one at a time. A host may drop a queued run; a tag job's
  drop leaves the tag where it was until a later run moves it, and so does a tag job that fails
  between images. Publishing is a job of its own, outside the group, since every commit on main
  still needs its images.
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
- the most recent few images that have only a build sha tag.

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
  treefmt-nix doesn't have, for `treefmt.nix` to enable.

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
- **Outside the sandbox**: the Rust tests run in the dev shell, not in `nix flake check`, so they
  can reach Docker and the project's other tools.
- **Nix tests**: a flake check named `test-*` is a test of the nix code, and `just test` builds it
  with the rest.
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
