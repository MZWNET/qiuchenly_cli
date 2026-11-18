pub mod aria2;

use crate::config;
use crate::macos::archive;
use crate::store::{catalog, client::Client};
use anyhow::{Context, Result};
use aria2::Downloader;
use std::fs;
use std::path::{Path, PathBuf};

pub async fn download_file(
    downloader: &Downloader,
    url: &str,
    output_dir: Option<&Path>,
    file_name: Option<&str>,
) -> Result<PathBuf> {
    let dir = match output_dir {
        Some(dir) => dir.to_path_buf(),
        None => config::download_dir(&config::load()?)?,
    };
    fs::create_dir_all(&dir).with_context(|| format!("创建下载目录 {}", dir.display()))?;
    downloader.download(url, &dir, file_name).await
}

pub async fn download_app(
    client: &Client,
    downloader: &Downloader,
    app: &str,
    version: Option<&str>,
    output_dir: Option<&Path>,
) -> Result<PathBuf> {
    let (_, version) = catalog::resolve_version(client, app, version).await?;
    download_file(downloader, &version.url, output_dir, None).await
}

pub async fn install_app(
    client: &Client,
    downloader: &Downloader,
    app: &str,
    version: Option<&str>,
    destination: &Path,
    keep: bool,
) -> Result<PathBuf> {
    let downloaded = download_app(client, downloader, app, version, None).await?;
    let installed = archive::install(&downloaded, destination)?;
    if !keep {
        fs::remove_file(&downloaded)
            .with_context(|| format!("安装成功，但删除下载文件 {} 失败", downloaded.display()))?;
    }

    Ok(installed)
}
