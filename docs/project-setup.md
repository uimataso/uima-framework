# Project setup

What a uima project's repo has around its code, and how to set it up. This repo follows it, so its
own files are the working example.

## Formatting

Every file in the repo is formatted, not only Rust, with [treefmt] through [treefmt-nix].

- **`treefmt.nix`** at the repo root lists the formatters. Start from this repo's
  [`treefmt.nix`](../treefmt.nix) and add a formatter for each other kind of file the project has.
- **rustfmt** is the one the project's `rust-toolchain.toml` pins, so the formatting doesn't
  change with whichever rustfmt is on `PATH`. It runs with its defaults: no `rustfmt.toml`.
- **`flake.nix`** turns `treefmt.nix` into the `formatter` (`nix fmt`), a `formatting` check, and a
  `treefmt` in the dev shell. See `treefmtFor` and its uses in this repo's
  [`flake.nix`](../flake.nix).
- **`justfile`**: `fmt` runs `treefmt`, and `fmt-check` runs `treefmt --fail-on-change` as part of
  `check`. `nix flake check` checks the same.

When adding formatting to an existing repo, format everything in a commit of its own and list it
in `.git-blame-ignore-revs`. GitHub and Forgejo read the file on their own; locally, run
`git config blame.ignoreRevsFile .git-blame-ignore-revs` once per clone.

```sh
# .git-blame-ignore-revs
# style: format every file with treefmt
<full commit sha>
```

## Lints

Clippy's default lints, plus a few more that the project's `Cargo.toml` turns on.

- **The lint table**: copy this repo's `[workspace.lints]` from [`Cargo.toml`](../Cargo.toml).
  Cargo can't share lints across repos, so each project keeps its own copy. Keep the lints at
  `warn`, not `deny`, so code in progress still builds.
- **Workspaces**: the table goes in `[workspace.lints]`, and every crate opts in with
  `[lints] workspace = true`; without that, the crate gets none of them. A single-crate project
  puts the table in `[lints]`.
- **`justfile`**: `lint` runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  as part of `check`. `--all-features` lints the code behind the `testing` feature too. The flake's
  check runs the same command.
- **Silencing a lint**: prefer `#[expect(lint, reason = "...")]` to `#[allow(...)]`. Once the
  lint no longer fires, `expect` warns, so the attribute doesn't outlive its reason.

[treefmt]: https://github.com/numtide/treefmt
[treefmt-nix]: https://github.com/numtide/treefmt-nix
