{pkgs, ...}: {
  languages.rust = {
    enable = true;
    channel = "nightly";
    lsp.enable = true;
  };

  git-hooks.hooks = {
    rustfmt.enable = true;
    clippy.enable = true;
  };

  packages = with pkgs; [
    openssl
    pkg-config
  ];

  enterTest = ''
    rustc --version
    cargo --version
    rustfmt --version
    clippy-driver --version
    rust-analyzer --version
  '';
}
