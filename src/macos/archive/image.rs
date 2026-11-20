use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

pub(crate) struct MountedImage {
    mountpoint: Option<TempDir>,
}

impl MountedImage {
    pub(crate) fn attach(archive: &Path) -> Result<Self> {
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

    pub(crate) fn detach(&mut self) -> Result<()> {
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

    pub(crate) fn path(&self) -> &Path {
        self.mountpoint.as_ref().expect("磁盘映像必须已挂载").path()
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
