{
  lib,
  rustPlatform,
  makeWrapper,
  aria2,
}:
rustPlatform.buildRustPackage {
  pname = "qiuchenly-cli";
  inherit ((lib.importTOML ./Cargo.toml).package) version;

  preBuild = "unset AWS_LC_SYS_USE_SYSTEM LIBSQLITE3_SYS_USE_PKG_CONFIG";
  nativeBuildInputs = [makeWrapper];
  postFixup = ''
    wrapProgram $out/bin/qiuchenly \
      --prefix PATH : ${lib.makeBinPath [aria2]}
  '';

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
