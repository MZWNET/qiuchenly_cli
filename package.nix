{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "qiuchenly-cli";
  inherit ((lib.importTOML ./Cargo.toml).package) version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./Cargo.lock
      ./Cargo.toml
      ./src
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  meta = {
    description = "A CLI reimplementation of the Qiuchenly App Store, written in Rust";
    homepage = "https://github.com/MZWNET/qiuchenly_cli";
    mainProgram = "qiuchenly";
  };
}
