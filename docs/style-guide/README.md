# Style Guide

How Rust is written in uima-framework and every service built on it.

Run `just fmt` before committing; `just fmt-check` (and CI) fails on unformatted files.

- [Cargo](cargo.md): features and `Cargo.toml`.
- [Conventions](conventions.md): how code is organized and written.
- [Testing](testing.md): how tests are organized and written.

How this repo's own crates are laid out and named is in [docs/crates.md](../crates.md).

## Resources

General background reading, not specific to this guide; where they disagree with it, this guide
wins:

- [Canonical's Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html)
- [Rust API guidelines](https://rust-lang.github.io/api-guidelines/about.html)
