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

      # A clean checkout exposes `self.rev`, a dirty one exposes only
      # `self.dirtyRev` — which Nix formats as `<commit>-dirty` — and a source
      # that is not a git tree exposes neither. The CDP revision contract in
      # `moli-protocol/src/version.rs` accepts a bare 40- or 64-digit hex hash
      # only, so drop the suffix and fall back to git's all-zero "no such
      # commit" sentinel rather than a word that cannot parse as a hash.
      nullRev = "0000000000000000000000000000000000000000";
      revision = nixpkgs.lib.removeSuffix "-dirty" (self.rev or self.dirtyRev or nullRev);
    in
    {
      packages = forEachSystem (pkgs: rec {
        moli = pkgs.callPackage ./nix/package.nix {
          src = self;
          inherit revision;
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
