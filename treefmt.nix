# Every formatter for the repo, run by `nix fmt` / `just fmt` and checked
# by `nix flake check`. rustfmt is set to the toolchain rust-toolchain.toml
# pins, and cargo-sort is defined, by the framework's nix/lib.nix.
#
# Programs are in alphabetical order. Only cargo-sort and taplo match the
# same files (Cargo.toml); cargo-sort runs first.
{ ... }:
{
  projectRootFile = "flake.nix";

  programs.cargo-sort.enable = true;

  programs.nixfmt.enable = true;

  programs.prettier = {
    enable = true;
    includes = [ "*.md" ];
    settings.proseWrap = "preserve";
  };

  programs.rustfmt = {
    enable = true;
    edition = "2024";
  };

  programs.taplo = {
    enable = true;
    priority = 1;
  };

  programs.yamlfmt = {
    enable = true;
    settings.formatter.retain_line_breaks = true;
  };
}
