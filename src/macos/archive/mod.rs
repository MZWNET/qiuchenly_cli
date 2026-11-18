mod image;
mod publish;

use anyhow::{bail, Context, Result};
use image::MountedImage;
use publish::publish;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

fn find_app(root: &Path) -> Result<PathBuf> {
    let mut found = None;
    let mut entries = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter();
    while let Some(entry) = entries.next() {
        let entry = entry?;
        if entry.file_type().is_dir() && entry.path().extension().is_some_and(|ext| ext == "app") {
            entries.skip_current_dir();
            if found.is_some() {
                bail!("安装包中包含多个 .app");
            }
            found = Some(entry.into_path());
        }
    }

    found.context("安装包中未找到 .app")
}

fn copy_app(source: &Path, stage: &Path) -> Result<PathBuf> {
    let staged = stage.join(source.file_name().expect("应用路径必须有文件名"));
    let output = Command::new("ditto")
        .arg(source)
        .arg(&staged)
        .output()
        .context("运行 ditto 复制应用")?;
    if !output.status.success() {
        bail!(
            "复制 {} 失败: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(staged)
}

pub fn install(archive: &Path, destination: &Path) -> Result<PathBuf> {
    fs::create_dir_all(destination)
        .with_context(|| format!("创建安装目录 {}", destination.display()))?;
    let stage = tempfile::tempdir_in(destination)
        .with_context(|| format!("在 {} 创建暂存目录", destination.display()))?;
    let extension = archive
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let staged = if extension.eq_ignore_ascii_case("zip") {
        let extraction = tempfile::tempdir().context("创建解压目录")?;
        let output = Command::new("ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(extraction.path())
            .output()
            .context("运行 ditto 解压")?;
        if !output.status.success() {
            bail!(
                "解压 {} 失败: {}",
                archive.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        copy_app(&find_app(extraction.path())?, stage.path())?
    } else if extension.eq_ignore_ascii_case("dmg") {
        let mut image = MountedImage::attach(archive)?;
        let staged = copy_app(
            &find_app(
                image
                    .mountpoint
                    .as_ref()
                    .expect("磁盘映像必须已挂载")
                    .path(),
            )?,
            stage.path(),
        )?;
        image.detach()?;
        staged
    } else {
        bail!(
            "不支持的安装包格式: {}（仅支持 .zip / .dmg）",
            archive.display()
        );
    };
    publish(stage, &staged, destination)
}
