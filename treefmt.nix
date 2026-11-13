# Every formatter for the repo, run by `nix fmt` / `just fmt` and checked
# by `nix flake check`. `rustfmt` comes in from flake.nix, so it is the
# toolchain rust-toolchain.toml pins.
#
# Programs are in alphabetical order. Only cargo-sort and taplo match the
# same files (Cargo.toml); `priority` runs cargo-sort first.
{ rustfmt }:
{ pkgs, ... }:
{
  projectRootFile = "flake.nix";

  # Not a treefmt-nix program, so it is set up by hand. cargo-sort takes
  # crate directories, not files, so the wrapper passes each file's
  # directory. `--grouped` sorts within each blank-line-separated block and
  # keeps the blocks.
  settings.formatter.cargo-sort = {
    command = pkgs.writeShellApplication {
      name = "cargo-sort-files";
      runtimeInputs = [ pkgs.cargo-sort ];
      text = ''
        for file in "$@"; do
          cargo-sort --grouped "$(dirname "$file")"
        done
      '';
    };
    includes = [ "Cargo.toml" ];
    priority = 0;
  };

  programs.nixfmt.enable = true;

  programs.prettier = {
    enable = true;
    includes = [ "*.md" ];
    settings.proseWrap = "preserve";
  };

  programs.rustfmt = {
    enable = true;
    package = rustfmt;
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
