{
  description = "Moli: a lite, fast, high-compatibility headless browser for AI agents";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forEachSystem (pkgs: rec {
        moli = pkgs.callPackage ./nix/package.nix {
          src = self;
          revision = self.rev or self.dirtyRev or "unknown";
        };
        default = moli;
      });

      devShells = forEachSystem (pkgs: {
        default = pkgs.callPackage ./nix/shell.nix { };
      });

      # `nixfmt` formats files, not directories, so `nix fmt` needs a wrapper
      # that walks the tree it is handed.
      formatter = forEachSystem (
        pkgs:
        pkgs.writeShellApplication {
          name = "nixfmt-tree";
          runtimeInputs = [
            pkgs.findutils
            pkgs.nixfmt
          ];
          text = ''find "''${@:-.}" -type f -name '*.nix' -exec nixfmt {} +'';
        }
      );
    };
}
