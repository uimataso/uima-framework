# Testing

## Where tests live

A module's tests live in its `tests` submodule. Start inline and move to a file only as the suite
grows:

| Layout                                | Use when                                                           | Example for module `foo`           |
| ------------------------------------- | ------------------------------------------------------------------ | ---------------------------------- |
| Inline `mod tests { ... }`            | Default: the suite is small enough to sit at the end of the file   | `mod tests` in `src/foo.rs`        |
| `tests.rs`                            | One module's suite is too big to stay inline                       | `src/foo/tests.rs`                 |
| `tests/` (`mod.rs` plus `<topic>.rs`) | One module's tests need splitting into several files by topic      | `src/foo/tests/{mod,<topic>}.rs`   |
| `testing`                             | Not tests: shared test helpers and doubles                         | `mod testing` in `src/foo.rs`      |
| `contract`                            | Not tests: a port's test suite that every adapter runs             | `pub mod contract` in `src/foo.rs` |
| `tests/` at the crate root            | Only for code a crate can't test from inside, such as a proc macro | n/a                                |
| `testdata/` at the crate root         | Not code: files a test reads, such as migrations, keys or media    | `testdata/migrations/`             |

- `tests.rs` holds the module body directly; don't wrap it in another `mod tests { ... }`. When it
  grows into `tests/`, move it to `tests/mod.rs`; never keep `tests.rs` and `tests/` side by side.
- Declare `#[cfg(test)] mod tests;` at the top of the file with the other `mod` declarations; an
  inline `mod tests { ... }` goes at the end.
- Tests that span several modules go in the `tests` of their closest common parent, e.g. the crate
  root's `src/tests.rs` or `src/tests/<topic>.rs`. Name `<topic>` after the behavior or scenario,
  not after a submodule. Don't use sibling files named by topic (`<topic>_test.rs`) or `#[path]`.
- `testing` that other crates use is behind the `testing` feature; `testing` used only by its own
  crate is behind `cfg(test)`.
- `testing` and `contract` grow the same way as `tests`, inline first, then a file.
- A key or credential in `testdata/` is named `test-only-*`, so it reads as one no service trusts.

Assertions use `uima_testing::check!` and `uima_testing::assert!`, never `std::assert*!`, for their
output. A pattern match is `assert!(let Ok(user) = result)`.
Group related cases into one test function named for the behavior instead of one function per
case, e.g. `new_rejects_invalid_input` checking several inputs in a row.

## Test harness

A harness sets up what a test runs against, such as a service in a container, and lives in a
`testing` module. Each external service has one harness, in the crate for that service (e.g.
`TestPostgres` in `uima-postgres-sqlx`); a crate that needs several composes them into its own
harness.

```rust
#[tokio::test]
async fn saves_and_finds_a_user() {
    let postgres = TestPostgres::start(&MIGRATOR).await;

    let repo = PostgresUserRepo::new(postgres.isolated().await);
    // ...
}
```

- Name it `Test<Service>` for one external service (`TestPostgres`, `TestS3`) and `Test<Role>` for
  a composed one (`TestServices`, `TestApp`), never `<Service>TestContext` or `<Service>Container`.
- `start()` starts a service that runs for the rest of the test, and panics on failure. Call it at
  the top of the test function and share it between that function's cases. A harness with nothing
  running, e.g. one over fakes or in-memory implementations, uses `new()` instead. Either takes
  what most tests need as arguments, e.g. `start(&migrator)`, and a variant for a rarer setup
  named `<verb>_with_<what>()`, e.g. `start_with_empty()`.
- `isolated()`, when cases need their own resource, returns one, e.g. a client on a new database,
  and panics on failure like `start()`. If it clears a shared resource instead of creating a new
  one, say so in its doc, since the cases using it must then run one after another.
- Fields are private; expose what tests need through methods such as `client()` and `url()`.
- A composed harness holds the harnesses it starts. Its `start()` starts theirs, and its
  `isolated()` builds on theirs.

## Contract

A port's `contract` is the test suite every implementation of the port must pass. It is a
`pub mod contract` in the crate that defines the port, behind the `testing` feature. It exposes
`run_all`, which runs every case in order, each on its own value from `isolated`, through
`uima_testing::run_case`. It panics at the first failure, naming the case and the line in
`run_all` that ran it.

```rust
// in the crate that defines the port
pub mod contract {
    pub async fn run_all<C>(isolated: impl AsyncFn() -> C)
    where
        C: HasUnitOfWork + HasUserRepo,
        C::UoW: HasUserRepo,
    {
        run_case(&isolated, create_then_get_roundtrips).await;
        // ...
    }
}

// in an adapter crate
#[tokio::test]
async fn contract() {
    let postgres = TestPostgres::start(&MIGRATOR).await;
    contract::run_all(async || PostgresUserRepo::new(postgres.isolated().await)).await;
}
```

- Adapters, fakes included, call `run_all`. Add every new case to `run_all` so every adapter runs
  it.
- Cases take `ctx: &C` with `Has<Port>` bounds, so adding a port to one case changes only that
  case. A port without `#[has_port]` has no `Has<Port>`, so its cases take the port directly.
- Cases that only some adapters support, e.g. ones needing real concurrency or foreign keys, go in
  a submodule named for that capability (`contract::concurrency`, `contract::foreign_keys`) with its
  own `run_all`. An adapter calls the `run_all` of each capability it has.
