{ pkgs ? (
    let
      inherit (builtins) currentSystem fetchTree fromJSON readFile;
      lock = fromJSON (readFile ./flake.lock);
      inherit (lock.nodes) nixpkgs;
    in
    import (fetchTree nixpkgs.locked) {
      system = currentSystem;
    }
    ),
    sourceInfo ? {
      BUILD_GIT_HASH = "unknown";
      BUILD_GIT_DIRTY = "false";
      BUILD_GIT_DATE = "";
    }
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

  inherit (sourceInfo) BUILD_GIT_HASH BUILD_GIT_DIRTY BUILD_GIT_DATE;

  nativeBuildInputs = with pkgs; [
    installShellFiles
  ];

  doCheck = true;

  postInstall = ''
    installShellCompletion --cmd agent-run \
      --bash <("$out/bin/agent-run" completion bash) \
      --zsh <("$out/bin/agent-run" completion zsh)
  '';
}
