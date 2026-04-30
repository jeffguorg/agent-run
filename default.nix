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

let
  rustPlatform = pkgs.makeRustPlatform {
    cargo = pkgs.cargo;
    rustc = pkgs.rustc;
  };
in
rustPlatform.buildRustPackage {
  pname = "agent-run";
  version = "0.1.0";

  src = pkgs.lib.cleanSource ./.;
  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = with pkgs; [
    installShellFiles
    pkg-config
  ];

  doCheck = true;

  postInstall = ''
    installShellCompletion --cmd agent-run \
      --bash <("$out/bin/agent-run" completion bash) \
      --zsh <("$out/bin/agent-run" completion zsh)
  '';
}
