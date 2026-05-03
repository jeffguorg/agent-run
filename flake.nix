{
  description = "agent-run Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forEachSystem = f:
        nixpkgs.lib.genAttrs systems (system:
          f (import nixpkgs { inherit system; })
        );
    in
    {
      packages = forEachSystem (pkgs: {
        default = import ./default.nix {
          inherit pkgs;
          sourceInfo = {
            BUILD_GIT_HASH = pkgs.lib.substring 0 7 (self.rev or self.dirtyRev);
            BUILD_GIT_DIRTY = if (self.rev or null) == null then "true" else "false";
            BUILD_GIT_DATE = let d = self.lastModifiedDate; in if d != null then "${nixpkgs.lib.substring 0 4 d}-${nixpkgs.lib.substring 4 2 d}-${nixpkgs.lib.substring 6 2 d}" else "";
          };
        };
      });
      devShells = forEachSystem (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });
    };
}
