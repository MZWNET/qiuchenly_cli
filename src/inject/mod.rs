mod config;
mod macho;
mod signing;

use crate::macos::apps::{self, InstalledApp};
use anyhow::{bail, Context, Result};
use config::AppConfig;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const DEFAULT_BRIDGE: &str = "/Contents/Frameworks/";
const INJECT_REF: &str = "@executable_path/q";

fn locate(bundle_id: &str) -> Result<(AppConfig, InstalledApp)> {
    let entry = config::load()?
        .app_list
        .into_iter()
        .find(|entry| entry.matches(bundle_id))
        .with_context(|| format!("注入配置中没有 {bundle_id}"))?;
    let app = match &entry.app_base_locate {
        Some(path) => apps::parse_app_bundle(Path::new(path))?,
        None => apps::find_installed_app(bundle_id)?,
    };

    Ok((entry, app))
}

fn injection_target(config: &AppConfig, app: &InstalledApp) -> Option<PathBuf> {
    let file = config.inject_file.as_deref()?;
    Some(
        app.path
            .join(
                config
                    .bridge_file
                    .as_deref()
                    .unwrap_or(DEFAULT_BRIDGE)
                    .trim_matches('/'),
            )
            .join(file),
    )
}

pub fn list() -> Result<()> {
    let Some(cached) = config::load_cached()? else {
        bail!("没有本地注入列表，请先运行 qiuchenly inject --update");
    };
    let installed = apps::scan_installed_apps()?;
    println!("{:<45} {:<10} 注入方式", "bundleId", "已装");
    for entry in &cached.config.app_list {
        for pattern in entry.package_name.patterns() {
            if !pattern.contains('*') && !pattern.contains('-') {
                let is_installed = installed.iter().any(|app| app.bundle_id == *pattern);
                let method = if entry.keygen == Some(true) {
                    "keygen"
                } else if entry.link_ability.is_some() {
                    "linkAbility"
                } else {
                    "dylib"
                };
                println!(
                    "{:<45} {:<10} {}",
                    pattern,
                    if is_installed { "是" } else { "否" },
                    method
                );
            }
        }
    }

    Ok(())
}

pub async fn update(aria2c: PathBuf) -> Result<()> {
    let cached = config::load_cached()?;
    let current_version = cached
        .as_ref()
        .map_or("0", |cached| cached.store_version.as_str());
    let client = crate::store::client::Client::from_channel(crate::config::DEFAULT_CHANNEL)?;
    let update = client.check_gui_update(current_version).await?;
    let package = update
        .latest_package
        .context("商店没有返回最新 GUI 安装包")?;
    if cached
        .as_ref()
        .is_some_and(|c| c.store_version == package.version)
    {
        println!("注入列表已是最新（QiuChenAppStore {}）", package.version);

        return Ok(());
    }
    let response = client
        .download_gui_update(&package.version, &package)
        .await?;
    if response.version != package.version {
        bail!(
            "下载响应版本不匹配: 期望 {}，实际 {}",
            package.version,
            response.version
        );
    }
    let temp = tempfile::tempdir().context("创建临时目录")?;
    let downloader = crate::packages::aria2::Downloader::new(aria2c);
    let dmg = downloader
        .download(
            &response.download_url,
            temp.path(),
            response.file_name.as_deref(),
        )
        .await?;
    crate::packages::checksum::verify_sha256(&dmg, &response.sha256, "下载文件")?;
    let mut mounted = crate::macos::archive::image::MountedImage::attach(&dmg)?;
    let config_path = mounted.path().join(
        "QiuChenAppStore.app/Contents/Resources/HayakuDaemon.app/Contents/Resources/config.json",
    );
    let data = fs::read(&config_path).with_context(|| format!("读取 {}", config_path.display()))?;
    let config: config::InjectConfig = serde_json::from_slice(&data)
        .with_context(|| format!("解析 {} 失败", config_path.display()))?;
    mounted.detach()?;
    let count = config.app_list.len();
    let cached = config::CachedInjectList {
        store_version: package.version,
        config,
    };
    config::save_cached(&cached)?;
    println!(
        "注入列表已更新（QiuChenAppStore {}，{count} 个应用）",
        cached.store_version
    );

    Ok(())
}

pub fn inject_app(bundle_id: &str) -> Result<()> {
    let (config, app) = locate(bundle_id)?;
    if !config.allows_version(&app.version) {
        bail!(
            "版本 {} 不在 supportVersion/supportSubVersion 允许范围",
            app.version
        );
    }
    if config.keygen == Some(true) {
        bail!("keygen 模式的应用不支持注入（原版走 keygen 通道）");
    }
    let dylib_source = config::find_dylib_source()?;
    let dylib_name = config.dylib_select.as_deref().unwrap_or("CoreInject.dylib");
    let copy_dylib = config.need_copy_to_app_dir.unwrap_or(false);
    let targets = if let Some(target) = injection_target(&config, &app) {
        vec![target]
    } else {
        let main = app.path.join("Contents/MacOS");
        let mut targets = Vec::new();
        for entry in
            fs::read_dir(&main).with_context(|| format!("读取注入目标目录 {}", main.display()))?
        {
            let path = entry?.path();
            if path.is_file()
                && path.extension() != Some(OsStr::new("backup"))
                && !(copy_dylib && path.file_name() == Some(OsStr::new(dylib_name)))
            {
                targets.push(path);
            }
        }
        if targets.is_empty() {
            bail!("{} 里没有可注入的二进制", main.display());
        }
        targets.sort();
        targets
    };

    println!("注入 {} ...", app.path.display());
    for target in &targets {
        let mut backup_name = target.as_os_str().to_owned();
        backup_name.push(".backup");
        let backup = PathBuf::from(backup_name);
        if !backup.exists() {
            fs::copy(target, &backup)
                .with_context(|| format!("备份 {} 到 {}", target.display(), backup.display()))?;
        }
        let copied_dylib = if copy_dylib {
            let destination = target
                .parent()
                .expect("注入目标位于应用目录内")
                .join(dylib_name);
            fs::copy(&dylib_source, &destination)
                .with_context(|| format!("复制注入库到 {}", destination.display()))?;
            Some(destination)
        } else {
            None
        };
        let load_path = match &copied_dylib {
            Some(path) => path.to_str().context("注入库路径不是 UTF-8")?,
            None => INJECT_REF,
        };
        macho::patch_binary(target, load_path, config.link_ability.as_deref())?;
    }
    if config.no_sign_target != Some(true) {
        let entitlements =
            config.entitlements.is_some() || config.disable_library_validate == Some(true);
        signing::codesign(
            &app.path,
            config.deep_sign_app.unwrap_or(false),
            entitlements,
        )?;
    }
    for target in targets {
        println!("已注入: {}", target.display());
    }
    signing::reset_tcc(&config, bundle_id)?;
    println!("完成");

    Ok(())
}

pub fn restore_app(bundle_id: &str) -> Result<()> {
    let (config, app) = locate(bundle_id)?;
    let mut directories = vec![
        app.path.join("Contents/Frameworks"),
        app.path.join("Contents/MacOS"),
    ];
    if let Some(target) = injection_target(&config, &app) {
        let directory = target.parent().expect("注入目标位于应用目录内");
        if !directories.iter().any(|path| path == directory) {
            directories.push(directory.to_path_buf());
        }
    }
    let mut restored = 0;
    for directory in directories {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("读取备份目录 {}", directory.display()))
            }
        };
        for entry in entries {
            let backup = entry?.path();
            if backup.extension() == Some(OsStr::new("backup")) {
                let original = backup.with_extension("");
                fs::copy(&backup, &original).with_context(|| {
                    format!("从 {} 还原 {}", backup.display(), original.display())
                })?;
                fs::remove_file(&backup)
                    .with_context(|| format!("删除已还原的备份 {}", backup.display()))?;
                restored += 1;
                println!("已还原: {}", original.display());
            }
        }
    }
    if restored == 0 {
        println!("{bundle_id} 没有找到备份");

        return Ok(());
    }
    signing::codesign(&app.path, true, false)?;
    println!("完成");

    Ok(())
}
