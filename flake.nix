{
  description = "uwoniri: Dynamic window resizing for niri on ultrawide monitors";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        uwoniri = pkgs.callPackage ./package.nix { };
        default = uwoniri;
      });

      overlays.default = final: _prev: {
        uwoniri = final.callPackage ./package.nix { };
      };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
          packages = with pkgs; [
            rust-analyzer
            clippy
            rustfmt
          ];
        };
      });
    };
}
