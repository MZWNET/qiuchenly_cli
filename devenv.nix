{pkgs, ...}: {
  languages.rust = {
    enable = true;
    channel = "nightly";
    lsp.enable = true;
  };

  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy.enable = true;
    alejandra = {
      enable = true;
      excludes = ["^Cargo\\.nix$"];
    };
  };

  packages = with pkgs; [
    crate2nix
    openssl
    pkg-config
  ];

  enterTest = ''
    rustc --version
    cargo --version
    rustfmt --version
    clippy-driver --version
    rust-analyzer --version
    alejandra --version
    crate2nix --version
  '';
}
