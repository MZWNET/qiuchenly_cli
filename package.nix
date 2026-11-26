{
  lib,
  pkgs,
  makeWrapper,
  aria2,
}: let
  cargoNix = import ./Cargo.nix {
    inherit pkgs;
    defaultCrateOverrides =
      pkgs.defaultCrateOverrides
      // {
        # The system aws-lc and SQLite break the build, so keep the vendored ones
        aws-lc-sys = _: {};
        libsqlite3-sys = _: {};
        qiuchenly-cli = _: {
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.lock
              ./Cargo.toml
              ./src
            ];
          };
        };
      };
  };
in
  cargoNix.rootCrate.build.overrideAttrs (old: {
    name = "qiuchenly-cli-${(lib.importTOML ./Cargo.toml).package.version}";

    # buildRustCrate's split "lib" output and the binary reference each other, a cycle Nix rejects
    outputs = ["out"];
    outputDev = ["out"];
    installPhase = ''
      runHook preInstall
      mkdir -p $out/bin
      cp -rP target/bin/* $out/bin/
      runHook postInstall
    '';

    nativeBuildInputs = (old.nativeBuildInputs or []) ++ [makeWrapper];
    postFixup = ''
      wrapProgram $out/bin/qiuchenly \
        --prefix PATH : ${lib.makeBinPath [aria2]}
    '';

    meta =
      old.meta
      // {
        description = "A CLI reimplementation of the Qiuchenly App Store, written in Rust";
        homepage = "https://github.com/MZWNET/qiuchenly_cli";
        mainProgram = "qiuchenly";
      };
  })
