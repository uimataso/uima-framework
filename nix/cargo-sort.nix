# cargo-sort as a treefmt-nix program, which treefmt-nix doesn't have.
# Enable it in treefmt.nix with `programs.cargo-sort.enable = true`.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.cargo-sort;
in
{
  options.programs.cargo-sort = {
    enable = lib.mkEnableOption "cargo-sort";
    priority = lib.mkOption {
      type = lib.types.int;
      default = 0;
      description = "Runs before taplo, which formats the same files.";
    };
  };

  config = lib.mkIf cfg.enable {
    # cargo-sort takes crate directories, not files, so the wrapper passes
    # each file's directory. `--grouped` sorts within each
    # blank-line-separated block and keeps the blocks.
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
      inherit (cfg) priority;
    };
  };
}
