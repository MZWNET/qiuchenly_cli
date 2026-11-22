# qiuchenly_cli

A CLI reimplementation of the Qiuchenly App Store, written in Rust.

## Motivation

Recent Qiuchenly App Store bundled too many functions far away the scope of App Store, like Proxy and Video Player. Also, the GUI App Store is not so friendly for scripts, for example, when we want all softwares get managed by nix-darwin.

So here I made this.

## Caution

This repo is not affiliated with Qiuchenly. Use it at your own risk!

## Requirements

- File downloads require `aria2c`:

```sh
brew install aria2
```

## Installation

- Installing from source requires the Rust toolchain. The resulting binary is `qiuchenly`:

```sh
cargo install --path .
```

- Automated builds are also available via GitHub Actions.

- Nix: `nix profile add github:MZWNET/qiuchenly_cli`, or use the flake's `overlays.default` (`pkgs.qiuchenly-cli`).

## Common Commands

```sh
# Search for apps and view all official versions
qiuchenly search <APP>
qiuchenly info <APP> --versions

# Get the download URL or install an app directly (optional install directory, defaults to /Applications)
qiuchenly download <APP> --url
qiuchenly install <APP> --appdir ~/Applications

# Install Adobe Photoshop, optionally with the Chinese language pack
qiuchenly install PHSP
qiuchenly install PHSP --language zh_CN

# Automatic version selection picks the latest stable release; --beta lets it pick betas,
# an explicit --version is never filtered (outdated supports --beta too)
qiuchenly download <APP> --version <VERSION>
qiuchenly install <APP> --beta

# Download or install the pre-cracked package instead of the official one (requires login)
qiuchenly install <APP> --precracked

# List installed apps and check for available updates, including the original GUI when installed
qiuchenly list --all
qiuchenly outdated

# Uninstall an app and clean up leftovers (TARGET is a Bundle ID or .app path)
qiuchenly uninstall <BUNDLE_ID> --dry-run

# Refresh the cached injectable-app list (downloads only when a newer store version is available); --list shows supported apps without the GUI installed
qiuchenly inject --update
qiuchenly inject --list

# Manage named API sources; the built-in default is used unless --channel selects another
# BASE_URL is an API root compatible with the default https://api1.xkji.com/api/v1 contract
qiuchenly channel list
qiuchenly channel add <NAME> <BASE_URL>
qiuchenly search <APP> --channel <NAME>
qiuchenly channel remove <NAME>
```

## Account & Additional Features

Login is supported.

In scripts, you can use `--password-stdin` to read the password from standard input:

```sh
qiuchenly login <USERNAME>
printf '%s\n' "\$PASSWORD" | qiuchenly login <USERNAME> --password-stdin
qiuchenly whoami
qiuchenly logout
```

Other subcommands include browsing categories (`categories`), popular apps (`popular`), orphaned leftover cleanup (`cleanup`), uninstall history and rules (`history`, `rules`), and app injection (`inject`), where injecting or restoring an app reads existing local Hayaku configurations and resources.

Run `qiuchenly --help` or `qiuchenly <COMMAND> --help` for full command and option details.
