# uima-framework

The common base for uima's services: shared crates, one style guide, and the nix and CI setup they
all build with.

- Style guide: [docs/style-guide/](docs/style-guide/README.md)
- Crate layout: [docs/crates.md](docs/crates.md)
- Agent and contributor rules: [AGENTS.md](AGENTS.md)

## Development

```sh
nix develop        # or: direnv allow
just check         # formatting + lints
just test          # every test
just ci            # nix flake check, then the tests, as CI runs them
```
