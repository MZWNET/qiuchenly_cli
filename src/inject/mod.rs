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

pub fn list() -> Result<()> {
    let config = config::load()?;
    let installed = apps::scan_installed_apps()?;
    println!("{:<45} {:<10} 注入方式", "bundleId", "已装");
    for entry in &config.app_list {
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
    let bridge = app.path.join(
        config
            .bridge_file
            .as_deref()
            .unwrap_or(DEFAULT_BRIDGE)
            .trim_matches('/'),
    );
    let copy_dylib = config.need_copy_to_app_dir.unwrap_or(false);
    let targets = if let Some(file) = &config.inject_file {
        vec![bridge.join(file)]
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
    if let Some(file) = &config.inject_file {
        let target = app
            .path
            .join(
                config
                    .bridge_file
                    .as_deref()
                    .unwrap_or(DEFAULT_BRIDGE)
                    .trim_matches('/'),
            )
            .join(file);
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
