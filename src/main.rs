mod cli;
mod config;
mod inject;
mod macos;
mod packages;
mod rules;
mod store;
mod uninstall;

use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL)
    };

    cli::Cli::parse().run().await
}
