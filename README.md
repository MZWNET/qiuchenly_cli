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

## Common Commands

```sh
# Search for apps and view all official versions
qiuchenly search <APP>
qiuchenly info <APP> --versions

# Get the download URL or install an app directly (optional install directory, defaults to /Applications)
qiuchenly download <APP> --url
qiuchenly install <APP> --appdir ~/Applications

# List installed apps and check for available updates
qiuchenly list --all
qiuchenly outdated

# Uninstall an app and clean up leftovers (TARGET is a Bundle ID or .app path)
qiuchenly uninstall <BUNDLE_ID> --dry-run
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

Other subcommands include browsing categories (`categories`), popular apps (`popular`), community releases (`community`), orphaned leftover cleanup (`cleanup`), uninstall history and rules (`history`, `rules`), and app injection (`inject`) that reads existing local Hayaku configurations and resources.

Run `qiuchenly --help` or `qiuchenly <COMMAND> --help` for full command and option details.
