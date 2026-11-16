use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;
use walkdir::WalkDir;

struct MountedImage {
    mountpoint: Option<TempDir>,
}

impl MountedImage {
    fn attach(archive: &Path) -> Result<Self> {
        let mountpoint = tempfile::tempdir().context("创建磁盘映像挂载点")?;
        let output = Command::new("hdiutil")
            .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
            .arg(mountpoint.path())
            .arg(archive)
            .output()
            .context("运行 hdiutil attach")?;
        if !output.status.success() {
            bail!(
                "挂载 {} 失败: {}",
                archive.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(Self {
            mountpoint: Some(mountpoint),
        })
    }

    fn detach(&mut self) -> Result<()> {
        let mountpoint = self.mountpoint.as_ref().expect("磁盘映像必须已挂载");
        let output = Command::new("hdiutil")
            .arg("detach")
            .arg(mountpoint.path())
            .output()
            .context("运行 hdiutil detach")?;
        if !output.status.success() {
            bail!(
                "卸载 {} 失败: {}",
                mountpoint.path().display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        drop(self.mountpoint.take());

        Ok(())
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        if let Some(mountpoint) = self.mountpoint.take() {
            let result = Command::new("hdiutil")
                .args(["detach", "-force"])
                .arg(mountpoint.path())
                .output();
            match result {
                Ok(output) if output.status.success() => {}
                Ok(output) => {
                    let path = mountpoint.keep();
                    eprintln!(
                        "强制卸载失败，挂载点保留在 {}: {}",
                        path.display(),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                Err(error) => {
                    let path = mountpoint.keep();
                    eprintln!(
                        "运行 hdiutil detach 失败，挂载点保留在 {}: {error}",
                        path.display()
                    );
                }
            }
        }
    }
}

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

fn publish(stage: TempDir, staged: &Path, destination: &Path) -> Result<PathBuf> {
    let target = destination.join(staged.file_name().expect("暂存应用路径必须有文件名"));
    let backup = stage.path().join("backup");
    let existing = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error).with_context(|| format!("读取 {}", target.display())),
    };
    if existing {
        fs::rename(&target, &backup).with_context(|| format!("备份 {}", target.display()))?;
    }
    if let Err(error) = fs::rename(staged, &target) {
        if existing {
            if let Err(restore_error) = fs::rename(&backup, &target) {
                let saved = stage.keep();
                bail!(
                    "安装 {} 失败: {error}；恢复旧应用失败: {restore_error}，旧应用保存在 {}",
                    target.display(),
                    saved.join("backup").display()
                );
            }
        }
        return Err(error).with_context(|| format!("安装 {}", target.display()));
    }

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::publish;
    use std::fs;

    #[test]
    fn publication_replaces_app_and_restores_it_after_rename_failure() {
        let destination = tempfile::tempdir().unwrap();
        let target = destination.path().join("Example.app");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("obsolete"), "old").unwrap();

        let stage = tempfile::tempdir_in(destination.path()).unwrap();
        let staged = stage.path().join("Example.app");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("current"), "new").unwrap();
        assert_eq!(publish(stage, &staged, destination.path()).unwrap(), target);
        assert!(!target.join("obsolete").exists());
        assert_eq!(fs::read_to_string(target.join("current")).unwrap(), "new");

        let stage = tempfile::tempdir_in(destination.path()).unwrap();
        let missing = stage.path().join("Example.app");
        assert!(publish(stage, &missing, destination.path()).is_err());
        assert_eq!(fs::read_to_string(target.join("current")).unwrap(), "new");
    }
}
