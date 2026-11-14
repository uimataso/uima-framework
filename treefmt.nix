# Every formatter for the repo; see docs/project-setup.md#formatting.
# Programs are in alphabetical order.
{ ... }:
{
  projectRootFile = "flake.nix";
  # A fixture that must stay unsorted; see nix/tests.nix.
  settings.global.excludes = [ "nix/tests/unsorted/*" ];

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

  programs.taplo.enable = true;

  programs.yamlfmt = {
    enable = true;
    settings.formatter.retain_line_breaks = true;
  };
}
