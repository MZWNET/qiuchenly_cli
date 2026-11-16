use anyhow::Result;
use std::path::Path;

use crate::config;
use crate::download::{self, aria2::Downloader};
use crate::store::{
    catalog::{resolve_bundle_id, resolve_version},
    client::Client,
    community,
};

use super::args::{OutputArgs, TransferArgs};

pub async fn download(
    app: &str,
    version: Option<&str>,
    url: bool,
    output: OutputArgs,
) -> Result<()> {
    let client = Client::from_config()?;
    if url {
        let (_, version) = resolve_version(&client, app, version).await?;
        println!("{}", version.url);
    } else {
        let downloader = Downloader::new(output.transfer.aria2c);
        let path = download::download_app(
            &client,
            &downloader,
            app,
            version,
            output.output_dir.as_deref(),
        )
        .await?;
        println!("下载完成: {}", path.display());
    }

    Ok(())
}

pub async fn install(
    app: &str,
    version: Option<&str>,
    appdir: &Path,
    keep: bool,
    transfer: TransferArgs,
) -> Result<()> {
    let client = Client::from_config()?;
    let downloader = Downloader::new(transfer.aria2c);
    let path = download::install_app(&client, &downloader, app, version, appdir, keep).await?;
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
    arch: Option<&str>,
    attachment: bool,
    output: OutputArgs,
) -> Result<()> {
    let client = Client::authenticated()?;
    let bundle_id = resolve_bundle_id(&client, app).await?;
    let file = community::resolve_download(&client, &bundle_id, release, arch, attachment).await?;
    let output_dir = match output.output_dir {
        Some(path) => path,
        None => config::download_dir(&config::load()?)?,
    };
    std::fs::create_dir_all(&output_dir)?;
    let downloader = Downloader::new(output.transfer.aria2c);
    let path = downloader
        .download(&file.url, &output_dir, file.file_name.as_deref())
        .await?;
    println!("下载完成: {}", path.display());

    Ok(())
}
