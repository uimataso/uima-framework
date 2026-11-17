# Crates

How the crates in this repo are laid out and named. For how to write each crate's `Cargo.toml`
and code, see the [style guide](style-guide/README.md).

## Roles

Every crate has one role. A crate is **inside** when an app's use cases use it directly, and
**edge** otherwise; the edge crates split into three roles by what they talk to.

| Role   | What it holds                                       | Depends on                     |
| ------ | --------------------------------------------------- | ------------------------------ |
| inside | domain types, ports and their contracts             | inside crates                  |
| client | the connection to one outside system, by a library  | `base/` crates, its library    |
| impl   | one port, implemented with one client               | that port's crate, that client |
| edge   | the rest of an app's edges: HTTP, config, logs, ... | anything but impl crates       |

Any crate may also depend on the base libraries below, and on another library that its public
API keeps hidden, as `cancel` does `tokio-util`.

- **An inside crate connects to no outside system**, such as a database or a network service, so
  it is light.
- **A port and its contract are inside; its implementations are impl crates**, each running the
  contract from the port crate's `testing` feature. An implementation that needs no library beyond
  what the port crate has stays in the port crate, as `SystemClock` does in `time`.
- **A client knows no port, and a port knows no client.** A concept of the outside system itself,
  such as a database transaction, stays in its client.

**Base libraries** are the libraries the whole framework commits to, as it does to std: `tokio`,
`serde`, `uuid`, `jiff`, `anyhow` and `tracing`. Any crate may use them in its public API, and no
crate's name names them. Replacing one is a breaking change across the framework, not a new crate
beside the old one. Every other library is an edge library.

## Directory and package name

Every crate lives two levels deep, under a group:

```sh
crates/
├── base/              # crates with a plain name
│   ├── core/          # uima-core
│   └── time/          # uima-time
├── client/            # clients
│   └── postgres-sqlx/ # uima-client-postgres-sqlx
└── http/              # the crates for one purpose
    └── axum/          # uima-http-axum
```

- A crate with a plain name goes in `base/`; a client in `client/`. A crate named for a library
  goes in a group named for its purpose, and a port gets a group named for it, holding the port
  crate and its impl crates.
- In `base/` the package is `uima-<dir>`. In any other group it is `uima-<group>-<dir>`, or
  `uima-<group>` when the directory has the group's name: a port `job` is `<port>/<port>`
  (`uima-job`), and its impl on `client/postgres-sqlx` is `job/postgres-sqlx`
  (`uima-job-postgres-sqlx`).
- A directory doesn't repeat the `uima-` prefix (`crates/base/core`, not `crates/base/uima-core`).
- `crates/` holds only groups and a group only crates, since `crates/*/*` makes every directory at
  that depth a workspace member.

## Naming

- **Built around an edge library: `<purpose>-<library>`.** A crate is built around a library when
  apps call it directly or the crate's public API is made of its types, as with
  `log-tracing-subscriber` (apps set up `tracing-subscriber`) and `http-axum` (apps build axum
  `Router`s). A library that only lends the crate a few types, as utoipa does `http-axum-api`,
  stays out of the name. A replacement library gets its own crate beside the old one,
  `<purpose>-<new>`, so apps on the old one keep getting fixes.
- **A client: `<system>-<library>`** in `client/`, as `client/postgres-sqlx`.
- **An impl: `<port>-<client>`**, in the port's group with its directory named for the client, as
  `job/postgres-sqlx` (`uima-job-postgres-sqlx`). An impl works with only one client, so a second
  client for the same system gets impl crates of its own.
- **Narrower use of another crate: `<base>-<what>`**, appended to the full name of the base.
  `http-axum-api` is the JSON API layer of `http-axum`.
- **Proc macros: `<name>-macro`**, named for the crate or concept whose items the macros generate
  code for. Generated code names its crate as `::uima_<crate>` by default and takes a
  `crate = <path>` attribute, as serde does, for an app that renames the dependency.
- **Otherwise a plain name** for what the crate does, also when its public API hides the library
  it uses, as `cancel` hides `tokio-util`.

## Light and heavy crates

Light crates keep an app that needs no database or OpenAPI from building them.

- Inside crates, and edge crates every service uses, are light, so they must not pull in a
  database driver, OpenAPI, or containers. Put those in their own heavy crate.
- A light crate depends only on other light crates, so nothing heavy comes in indirectly.

## Test helpers

`uima-testing` holds test helpers any crate can use, and depends on no other `uima-*` crate:
crates use it as a dev-dependency, so a dependency back on one of them would build that crate
twice with two incompatible copies of its types. Helpers tied to one crate go behind that crate's
`testing` feature instead.

## Depending on the framework

There is no facade crate re-exporting the others. An app lists the `uima-*` crates it uses once,
all at the same `rev`, in its root `[workspace.dependencies]` with its crates using
`{ workspace = true }`, or in `[dependencies]` for a single-crate app.

```toml
[workspace.dependencies]
uima-core = { git = "https://github.com/uimataso/uima-framework", rev = "<rev>" }
uima-log-tracing-subscriber = { git = "https://github.com/uimataso/uima-framework", rev = "<rev>" }
```

- **Toolchain:** the app's toolchain must be at least the framework's `rust-version`.
- **Local checkout:** put the `[patch]` in the app's `.cargo/config.toml` and don't commit it, so
  `nix build`, which only sees committed files, still uses the git `rev`. The patch also rewrites
  `Cargo.lock`; don't commit that either:

  ```toml
  [patch."https://github.com/uimataso/uima-framework"]
  uima-log-tracing-subscriber = { path = "../uima-framework/crates/log/tracing-subscriber" }
  ```
