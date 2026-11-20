pub mod aria2;

use crate::config;
use crate::macos::archive;
use crate::store::{catalog, client::Client, precracked};
use anyhow::{Context, Result};
use aria2::Downloader;
use std::fs;
use std::path::{Path, PathBuf};

pub struct ResolvedDownload {
    pub bundle_id: String,
    pub url: String,
    pub file_name: Option<String>,
}

pub struct InstallOptions<'a> {
    pub destination: &'a Path,
    pub language: Option<&'a str>,
    pub keep: bool,
}

pub async fn resolve_download(
    client: &Client,
    app: &str,
    version: Option<&str>,
    beta: bool,
    is_precracked: bool,
) -> Result<ResolvedDownload> {
    if is_precracked {
        let (bundle_id, download) =
            precracked::resolve_download(client, app, version, beta).await?;
        Ok(ResolvedDownload {
            bundle_id,
            url: download.download_url,
            file_name: download.file_name,
        })
    } else {
        let (bundle_id, version) = catalog::resolve_version(client, app, version, beta).await?;
        Ok(ResolvedDownload {
            bundle_id,
            url: version.url,
            file_name: None,
        })
    }
}

pub async fn download(
    downloader: &Downloader,
    resolved: &ResolvedDownload,
    output_dir: Option<&Path>,
) -> Result<PathBuf> {
    download_file(
        downloader,
        &resolved.url,
        output_dir,
        resolved.file_name.as_deref(),
    )
    .await
}

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

pub async fn install(
    client: &Client,
    downloader: &Downloader,
    app: &str,
    version: Option<&str>,
    beta: bool,
    is_precracked: bool,
    options: &InstallOptions<'_>,
) -> Result<PathBuf> {
    let resolved = match resolve_download(client, app, version, beta, is_precracked).await {
        Ok(resolved) => {
            if options.language.is_some() {
                anyhow::bail!("--language 仅适用于 Adobe 安装");
            }

            resolved
        }
        Err(err) if !is_precracked => {
            return match crate::adobe::install(downloader, app, version, beta, options).await {
                Ok(Some(path)) => Ok(path),
                Ok(None) => Err(err),
                Err(adobe_err) => Err(adobe_err.context(err)),
            };
        }
        Err(err) => return Err(err),
    };
    let downloaded = download(downloader, &resolved, None).await?;

    install_downloaded(
        &downloaded,
        options.destination,
        &resolved.bundle_id,
        options.keep,
    )
}

fn install_downloaded(
    downloaded: &Path,
    destination: &Path,
    bundle_id: &str,
    keep: bool,
) -> Result<PathBuf> {
    let installed = archive::install(downloaded, destination, bundle_id)?;
    if !keep {
        fs::remove_file(downloaded)
            .with_context(|| format!("安装成功，但删除下载文件 {} 失败", downloaded.display()))?;
    }

    Ok(installed)
}
