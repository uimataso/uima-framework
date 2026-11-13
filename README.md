# uima-framework

The common base for uima's services: shared crates, one style guide, and the nix and CI setup they
all build with.

- Style guide: [docs/style-guide/](docs/style-guide/README.md)
- Crate layout: [docs/crates.md](docs/crates.md)
- Agent and contributor rules: [AGENTS.md](AGENTS.md)

## Development

```sh
nix develop        # or: direnv allow
just check         # fmt + clippy + tests
just ci            # nix flake check, as CI runs it
```
