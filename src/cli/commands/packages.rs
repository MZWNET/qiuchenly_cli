use anyhow::Result;

use crate::packages::{self, aria2::Downloader, InstallOptions};
use crate::store::{catalog::resolve_bundle_id, client::Client, community};

use super::super::args::{InstallArgs, OutputArgs};

pub async fn download(
    app: &str,
    version: Option<&str>,
    beta: bool,
    precracked: bool,
    url: bool,
    output: OutputArgs,
) -> Result<()> {
    let client = if precracked {
        Client::authenticated()?
    } else {
        Client::from_config()?
    };
    let resolved = packages::resolve_download(&client, app, version, beta, precracked).await?;
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
    let client = if args.precracked {
        Client::authenticated()?
    } else {
        Client::from_config()?
    };
    let downloader = Downloader::new(args.transfer.aria2c.clone());
    let path = packages::install(
        &client,
        &downloader,
        &args.app,
        args.version.as_deref(),
        args.beta,
        args.precracked,
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

pub async fn community_list(app: &str) -> Result<()> {
    let client = Client::authenticated()?;
    let bundle_id = resolve_bundle_id(&client, app).await?;
    let releases = client.user_app_publishers(&bundle_id).await?;
    if releases.is_empty() {
        println!("{bundle_id} 没有社区发布版本");
    }
    for release in releases {
        let packages = release
            .packages
            .iter()
            .map(|package| {
                format!(
                    "{}({})",
                    package.architecture.as_deref().unwrap_or(&package.kind),
                    package.file_id
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{}  {:<12} {}{}  包: {}",
            release.id,
            release.version,
            release.full_version,
            if release.beta { " (beta)" } else { "" },
            packages
        );
    }

    Ok(())
}

pub async fn community_download(
    app: &str,
    release: Option<&str>,
    beta: bool,
    arch: Option<&str>,
    attachment: bool,
    output: OutputArgs,
) -> Result<()> {
    let client = Client::authenticated()?;
    let bundle_id = resolve_bundle_id(&client, app).await?;
    let file =
        community::resolve_download(&client, &bundle_id, release, arch, attachment, beta).await?;
    let downloader = Downloader::new(output.transfer.aria2c);
    let path = packages::download_file(
        &downloader,
        &file.url,
        output.output_dir.as_deref(),
        file.file_name.as_deref(),
    )
    .await?;
    println!("下载完成: {}", path.display());

    Ok(())
}
