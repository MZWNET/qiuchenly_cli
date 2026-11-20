mod application;
mod catalog;
mod condition;
mod database;
mod download;
mod installer;
mod model;
mod pimx;
mod response;
mod system;

use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::config;
use crate::packages::{aria2::Downloader, InstallOptions};

use catalog::CatalogClient;

pub async fn install(
    downloader: &Downloader,
    app: &str,
    version: Option<&str>,
    beta: bool,
    options: &InstallOptions<'_>,
) -> Result<Option<PathBuf>> {
    let client = CatalogClient::new()?;
    let Some(products) = client.resolve_products(app, version, beta).await? else {
        return Ok(None);
    };

    let main = products
        .last()
        .expect("resolve_products 返回的产品列表不为空");
    let staging_root = config::download_dir(&config::load()?)?.join("Adobe");
    let sap_dir = staging_root.join(&main.sap_code);
    fs::create_dir_all(&sap_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(&format!("{}-", main.version))
        .tempdir_in(&sap_dir)?;

    let language = options.language.unwrap_or("en_US");
    let mut downloaded = Vec::with_capacity(products.len());
    for product in &products {
        downloaded.push(
            download::download_product(&client, downloader, product, staging.path(), language)
                .await?,
        );
    }

    let installed = installer::install_products(&downloaded, options.destination, language)?;

    if options.keep {
        let path = staging.keep();
        println!("安装包已保留: {}", path.display());
    }

    Ok(Some(installed))
}
