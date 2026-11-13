# Cargo

## Features

A crate has at most one cargo feature, and it is `testing`.

- `testing` exposes what other crates need to test against this one: test harnesses
  (`Test<Service>`) and port contracts (`pub mod contract`). See [Testing](testing.md).
- Anything else optional is its own crate, not a feature. More features mean more combinations
  that the LSP and the test run each see only one of.

## Cargo.toml

Dependencies go in hand-picked blocks separated by blank lines. `just fmt` runs
`cargo sort --grouped`, which sorts within each block but keeps the blocks and their order, so
choosing the blocks is up to you:

```toml
[package]
name = "example"
version.workspace = true
rust-version.workspace = true
edition.workspace = true
authors.workspace = true

[features]
testing = ["dep:testcontainers"]

[dependencies]
# local crates
uima-cancel = { workspace = true }
uima-core = { workspace = true }

# workspace dependencies
anyhow = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["time"] }
tower = { workspace = true }
tracing = { workspace = true }

# crate-local dependencies
base64 = "0.22"

# testing-only, behind the `testing` feature
testcontainers = { workspace = true, optional = true }

[dev-dependencies]
assert2 = { workspace = true }

[lints]
workspace = true
```

- The root `[workspace.dependencies]` follows the same idea, wider: local crates first, then
  workspace dependencies split into a few blocks, with test- and dev-only dependencies last.
- A dependency must go in `[workspace.dependencies]` when crates need the same version of it:
  its types cross a crate boundary (`tokio`, `serde`, `jiff`, `axum`, `tower`), or it holds
  global state (`tracing`).
- Any other dependency can go either way. A workspace dependency keeps the version in one place,
  but a major version bump then has to happen in every crate at once; a crate-local one lets each
  crate bump on its own.
