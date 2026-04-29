{
  description = "agent-run Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      packages.${system}.default = import ./default.nix { inherit pkgs; };
      devShells.${system}.default = import ./shell.nix { inherit pkgs; };
    };
}
