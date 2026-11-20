pub(crate) mod image;
mod publish;

use anyhow::{bail, Context, Result};
use image::MountedImage;
use publish::publish;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[derive(Debug, PartialEq, Eq)]
enum InstallAction {
    CopyApp(PathBuf),
    RunInstaller(PathBuf),
}

enum PreparedInstall {
    Staged(tempfile::TempDir, PathBuf),
    Installed(PathBuf),
}

fn select_install_action(root: &Path, bundle_id: &str) -> Result<InstallAction> {
    let mut found_any = false;
    let mut target_app = None;
    let mut installer = None;
    let mut entries = WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter();
    while let Some(entry) = entries.next() {
        let entry = entry?;
        if entry.file_type().is_dir() && entry.path().extension().is_some_and(|ext| ext == "app") {
            entries.skip_current_dir();
            found_any = true;
            let app_bundle_id = crate::macos::apps::parse_app_bundle(entry.path())?.bundle_id;
            let name = entry.file_name();
            if app_bundle_id == bundle_id {
                if target_app.is_some() {
                    bail!("安装包中包含多个标识为 {bundle_id} 的 .app");
                }
                target_app = Some(entry.into_path());
            } else if name != "Uninstall.app"
                && (name.as_encoded_bytes().starts_with(b"Install")
                    || app_bundle_id.ends_with(".installer"))
            {
                if installer.is_some() {
                    bail!("安装包中包含多个厂商安装器，无法确定要运行的应用");
                }
                installer = Some(entry.into_path());
            }
        }
    }

    if !found_any {
        bail!("安装包中未找到 .app");
    }
    if let Some(installer) = installer {
        return Ok(InstallAction::RunInstaller(installer));
    }

    target_app
        .map(InstallAction::CopyApp)
        .with_context(|| format!("安装包中未找到标识为 {bundle_id} 的 .app"))
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

fn run_installer(path: &Path, bundle_id: &str) -> Result<PathBuf> {
    let output = Command::new("open")
        .arg("-W")
        .arg(path)
        .output()
        .context("运行厂商安装器")?;
    if !output.status.success() {
        bail!(
            "运行安装器 {} 失败: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(crate::macos::apps::find_installed_app(bundle_id)
        .with_context(|| format!("安装器已退出，但未找到目标应用 {bundle_id}"))?
        .path)
}

fn prepare_install(root: &Path, destination: &Path, bundle_id: &str) -> Result<PreparedInstall> {
    match select_install_action(root, bundle_id)? {
        InstallAction::CopyApp(app) => {
            let stage = tempfile::tempdir_in(destination)
                .with_context(|| format!("在 {} 创建暂存目录", destination.display()))?;
            let staged = copy_app(&app, stage.path())?;

            Ok(PreparedInstall::Staged(stage, staged))
        }
        InstallAction::RunInstaller(installer) => Ok(PreparedInstall::Installed(run_installer(
            &installer, bundle_id,
        )?)),
    }
}

pub fn install(archive: &Path, destination: &Path, bundle_id: &str) -> Result<PathBuf> {
    fs::create_dir_all(destination)
        .with_context(|| format!("创建安装目录 {}", destination.display()))?;
    let extension = archive
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let prepared = if extension.eq_ignore_ascii_case("zip") {
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
        prepare_install(extraction.path(), destination, bundle_id)?
    } else if extension.eq_ignore_ascii_case("dmg") {
        let mut image = MountedImage::attach(archive)?;
        let prepared = prepare_install(image.path(), destination, bundle_id)?;
        image.detach()?;
        prepared
    } else {
        bail!(
            "不支持的安装包格式: {}（仅支持 .zip / .dmg）",
            archive.display()
        );
    };
    match prepared {
        PreparedInstall::Staged(stage, staged) => publish(stage, &staged, destination),
        PreparedInstall::Installed(path) => Ok(path),
    }
}

#[cfg(test)]
mod tests {
    use super::{select_install_action, InstallAction};
    use plist::{Dictionary, Value};
    use std::fs;

    fn create_app(root: &std::path::Path, name: &str, bundle_id: &str) {
        let contents = root.join(name).join("Contents");
        fs::create_dir_all(&contents).unwrap();
        let mut info = Dictionary::new();
        info.insert("CFBundleIdentifier".into(), Value::String(bundle_id.into()));
        plist::to_file_binary(contents.join("Info.plist"), &info).unwrap();
    }

    #[test]
    fn selects_vendor_installer_over_target_app() {
        let dir = tempfile::tempdir().unwrap();
        create_app(dir.path(), "Install.app", "com.parallels.desktop.installer");
        create_app(
            dir.path(),
            "Parallels Desktop.app",
            "com.parallels.desktop.console",
        );
        assert_eq!(
            select_install_action(dir.path(), "com.parallels.desktop.console").unwrap(),
            InstallAction::RunInstaller(dir.path().join("Install.app"))
        );
    }

    #[test]
    fn selects_single_marked_installer() {
        let dir = tempfile::tempdir().unwrap();
        create_app(dir.path(), "Install.app", "com.example.setup");
        assert_eq!(
            select_install_action(dir.path(), "com.example.app").unwrap(),
            InstallAction::RunInstaller(dir.path().join("Install.app"))
        );
    }

    #[test]
    fn selects_single_matching_app_for_copy() {
        let dir = tempfile::tempdir().unwrap();
        create_app(dir.path(), "Example.app", "com.example.app");
        assert_eq!(
            select_install_action(dir.path(), "com.example.app").unwrap(),
            InstallAction::CopyApp(dir.path().join("Example.app"))
        );
    }
}
