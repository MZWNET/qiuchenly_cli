use anyhow::Result;

use crate::packages::{self, aria2::Downloader, InstallOptions};

use super::super::args::{InstallArgs, OutputArgs};

pub async fn download(
    app: &str,
    version: Option<&str>,
    beta: bool,
    precracked: bool,
    url: bool,
    output: OutputArgs,
    channel: &str,
) -> Result<()> {
    let resolved = packages::resolve_download(app, version, beta, precracked, channel).await?;
    if url {
        println!("{}", resolved.url);
    } else {
        let downloader = Downloader::new(output.transfer.aria2c);
        let path = packages::download(&downloader, &resolved, output.output_dir.as_deref()).await?;
        println!("下载完成: {}", path.display());
    }

    Ok(())
}

pub async fn install(args: &InstallArgs) -> Result<()> {
    if args.precracked && args.language.is_some() {
        anyhow::bail!("--language 仅适用于 Adobe 安装");
    }
    let downloader = Downloader::new(args.transfer.aria2c.clone());
    let path = packages::install(
        &downloader,
        &args.app,
        args.version.as_deref(),
        args.beta,
        args.precracked,
        &args.channel.channel,
        &InstallOptions {
            destination: &args.appdir,
            language: args.language.as_deref(),
            keep: args.keep_download,
        },
    )
    .await?;
    println!("安装完成: {}", path.display());

    Ok(())
}
