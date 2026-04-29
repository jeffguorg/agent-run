{ pkgs ? (
    let
      inherit (builtins) currentSystem fetchTree fromJSON readFile;
      lock = fromJSON (readFile ./flake.lock);
      inherit (lock.nodes) nixpkgs;
    in
    import (fetchTree nixpkgs.locked) {
      system = currentSystem;
    }
  )
}:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    clippy
    pkg-config
    rust-analyzer
    rustc
    rustfmt
    taplo
  ];

  RUST_BACKTRACE = "1";
}
