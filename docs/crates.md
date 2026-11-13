# Crates

How the crates in this repo are laid out and named. For how to write each crate's `Cargo.toml`
and code, see the [style guide](style-guide/README.md).

## Directory and package name

Every crate lives directly under `crates/`, one level deep:

```sh
crates/
├── core/           # uima-core
├── core-macro/     # uima-core-macro: the proc macros of core/
├── log-tracing/    # uima-log-tracing
├── http-axum/      # uima-http-axum
├── http-axum-api/  # uima-http-axum-api: the JSON API layer on top of http-axum/
└── ...
```

- The directory is `<name>`; the package is `uima-<name>`. The directory doesn't repeat the
  `uima-` prefix (`crates/core`, not `crates/uima-core`).
- `crates/` holds only crates, since `crates/*` makes every subdirectory a workspace member.

## Naming

- **Coupled to a library: `<purpose>-<library>`.** Apps are coupled to a library when they call it
  directly or the crate's public API exposes its types: `log-tracing` (apps write
  `tracing::info!`), `http-axum` (apps build axum `Router`s). A replacement library then gets its
  own crate beside it, `<purpose>-<new>`, so apps on the old one keep getting fixes. The
  `<purpose>` needn't be a crate of its own.
- **Narrower use of another crate: `<base>-<what>`**, appended to the full name of the base:
  `http-axum-api` is the JSON API layer of `http-axum`.
- **Proc macros: `<crate>-macro`**, for the crate whose items the macros generate code for. The
  generated code must not hard-code `::uima_<crate>::` paths, since an app may rename the
  dependency.
- **Otherwise a plain name** for what the crate does.

## Light and heavy crates

The services include small ones without a database that only serve HTML.

- A crate that every service uses is light: it must not pull in a database driver, OpenAPI, or
  containers. Put those in their own heavy crate.
- A light crate depends only on other light crates, so nothing heavy comes in indirectly.

## Test helpers

`uima-testing` holds test helpers any crate can use, and depends on no other `uima-*` crate:
crates use it as a dev-dependency, so a dependency back on one of them would build that crate
twice with two incompatible copies of its types. Helpers tied to one crate go behind that crate's
`testing` feature instead.

## Depending on the framework

There is no facade crate re-exporting the others. An app lists the `uima-*` crates it uses once,
all at the same `rev`: in its root `[workspace.dependencies]`, with its crates using
`{ workspace = true }`, or in `[dependencies]` for a single-crate app.

```toml
[workspace.dependencies]
uima-core = { git = "https://github.com/uimataso/uima-framework", rev = "<rev>" }
uima-log-tracing = { git = "https://github.com/uimataso/uima-framework", rev = "<rev>" }
```

- **Toolchain:** the app's toolchain must be at least the framework's `rust-version`.
- **Nix:** `importCargoLock` fetches the repo once per `rev` and needs its hash in
  `cargoLock.outputHashes`, under the name of any one crate from it
  (`"uima-log-tracing-0.0.0" = "sha256-...";`). After bumping `rev`, set it to `lib.fakeHash` and
  nix prints the right one.
- **Local checkout:** put the `[patch]` in the app's `.cargo/config.toml` and don't commit it, so
  `nix build`, which only sees committed files, still uses the git `rev`. The patch also rewrites
  `Cargo.lock`; don't commit that either:

  ```toml
  [patch."https://github.com/uimataso/uima-framework"]
  uima-log-tracing = { path = "../uima-framework/crates/log-tracing" }
  ```
