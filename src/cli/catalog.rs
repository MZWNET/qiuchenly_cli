use anyhow::{bail, Result};

use crate::macos::apps::scan_installed_apps;
use crate::store::{
    catalog::{is_newer, resolve_bundle_id},
    client::Client,
};

use super::args::PageArgs;

fn trunc(s: &str, n: usize) -> String {
    let mut chars = s.chars();
    let mut text: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() {
        text.push('…');
    }

    text
}

pub async fn search(query: &str, page: PageArgs) -> Result<()> {
    let client = Client::from_config()?;
    let result = client.search(query, page.page, page.limit).await?;
    println!("共 {} 条结果（第 {} 页）", result.total, page.page);
    for entry in result.list {
        println!(
            "{:<45} {:<30} {}",
            entry.bundle_id,
            trunc(&entry.name, 28),
            entry.category.first().map(String::as_str).unwrap_or("-")
        );
    }

    Ok(())
}

pub async fn categories(key: Option<String>, query: Option<String>, page: PageArgs) -> Result<()> {
    let client = Client::from_config()?;
    if let Some(key) = key {
        let result = client
            .category_apps(&key, page.page, page.limit, query.as_deref())
            .await?;
        println!("共 {} 个应用（第 {} 页）", result.total, page.page);
        for app in result.list {
            println!(
                "{:<45} {:<30} ★{:.1} ({})",
                app.bundle_id,
                trunc(&app.app_name, 28),
                app.rating,
                app.rating_count
            );
        }
    } else {
        for category in client.categories().await? {
            println!("{:<45} {}", category.key, category.name);
        }
    }

    Ok(())
}

pub async fn info(app: &str, all_versions: bool) -> Result<()> {
    let client = Client::from_config()?;
    let bundle_id = resolve_bundle_id(&client, app).await?;
    let result = client.search(&bundle_id, 1, 20).await?;
    if let Some(entry) = result
        .list
        .into_iter()
        .find(|entry| entry.bundle_id == bundle_id)
    {
        println!("名称:      {}", entry.name);
        println!("bundleId:  {}", entry.bundle_id);
        println!("seriesId:  {}", entry.series_id);
        if !entry.developer.is_empty() {
            println!("开发者:    {}", entry.developer.join(", "));
        }
        if !entry.category.is_empty() {
            println!("分类:      {}", entry.category.join(", "));
        }
        if !entry.architecture.is_empty() {
            println!("架构:      {}", entry.architecture.join(", "));
        }
        if !entry.introduction.is_empty() {
            println!("简介:");
            for line in entry.introduction.lines() {
                println!("  {line}");
            }
        }
    } else {
        println!("云端目录中未找到 {bundle_id}");
    }
    if let Some(local) = scan_installed_apps()?
        .into_iter()
        .find(|local| local.bundle_id == bundle_id)
    {
        println!("\n本地安装:  {}", local.path.display());
        println!("本地版本:  {}", local.version);
    }
    let versions = client.app_versions(&bundle_id).await?;
    if !versions.is_empty() {
        println!("\n官方版本:");
        let count = if all_versions { versions.len() } else { 5 };
        for version in versions.iter().take(count) {
            println!(
                "  {:<12} {}{}",
                version.version,
                if version.beta { "(beta) " } else { "" },
                version.url
            );
        }
    }

    Ok(())
}

pub async fn list(all: bool) -> Result<()> {
    let installed = scan_installed_apps()?;
    let client = if all {
        None
    } else {
        Some(Client::from_config()?)
    };
    let mut shown = 0;
    for app in installed {
        if let Some(client) = &client {
            let result = client.search(&app.bundle_id, 1, 20).await?;
            if !result
                .list
                .iter()
                .any(|entry| entry.bundle_id == app.bundle_id)
            {
                continue;
            }
        }
        println!(
            "{:<45} {:<28} {}",
            app.bundle_id,
            trunc(&app.name, 26),
            app.version
        );
        shown += 1;
    }
    if shown == 0 {
        println!("没有匹配的应用");
    } else {
        println!("共 {shown} 个");
    }

    Ok(())
}

pub async fn outdated(targets: Vec<String>) -> Result<()> {
    let installed = scan_installed_apps()?;
    let client = Client::from_config()?;
    let selected = !targets.is_empty();
    if selected {
        for target in &targets {
            let mut matches = installed.iter().filter(|app| {
                app.bundle_id.eq_ignore_ascii_case(target) || app.name.eq_ignore_ascii_case(target)
            });
            let Some(first) = matches.next() else {
                bail!("未安装应用：{target}");
            };
            if let Some(second) = matches.next() {
                let bundle_ids = std::iter::once(first)
                    .chain(std::iter::once(second))
                    .chain(matches)
                    .map(|app| app.bundle_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("应用名称 {target} 不明确，请指定 bundle ID：{bundle_ids}");
            }
        }
    }
    let mut shown = 0;
    for app in installed.iter().filter(|app| {
        !selected
            || targets.iter().any(|target| {
                app.bundle_id.eq_ignore_ascii_case(target) || app.name.eq_ignore_ascii_case(target)
            })
    }) {
        let versions = client.app_versions(&app.bundle_id).await?;
        let Some(latest) = versions.first() else {
            if selected {
                bail!("{} 没有官方版本信息", app.bundle_id);
            }
            continue;
        };
        if is_newer(&app.version, &latest.version)? {
            println!(
                "{}  {} -> {}  {}",
                app.bundle_id, app.version, latest.version, latest.url
            );
            shown += 1;
        }
    }
    if shown == 0 {
        println!("没有可用更新");
    }

    Ok(())
}

pub async fn popular() -> Result<()> {
    let client = Client::from_config()?;
    for app in client.popular().await? {
        println!(
            "{:<6} {:<40} {:<30} {}",
            app.download_count,
            app.bundle_id,
            trunc(&app.app_name, 28),
            app.version
        );
    }

    Ok(())
}

pub async fn store_version(current: &str) -> Result<()> {
    let client = Client::from_config()?;
    let update = client.check_gui_update(current).await?;
    if update.has_update {
        if let Some(package) = update.latest_package {
            println!("图形应用商店有新版本: {}", package.version);
            if let Some(notes) = package.release_notes {
                println!("{}", notes.replace('\n', " "));
            }
        } else {
            println!("图形应用商店有新版本");
        }
    } else {
        println!("图形应用商店已是最新版本（{current}）");
    }

    Ok(())
}
