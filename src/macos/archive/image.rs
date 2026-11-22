use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use super::super::process;

pub(crate) struct MountedImage {
    mountpoint: Option<TempDir>,
}

impl MountedImage {
    pub(crate) fn attach(archive: &Path) -> Result<Self> {
        let mountpoint = tempfile::tempdir().context("创建磁盘映像挂载点")?;
        process::run(
            Command::new("hdiutil")
                .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
                .arg(mountpoint.path())
                .arg(archive),
            &format!("挂载 {}", archive.display()),
        )?;

        Ok(Self {
            mountpoint: Some(mountpoint),
        })
    }

    pub(crate) fn detach(&mut self) -> Result<()> {
        let mountpoint = self.mountpoint.as_ref().expect("磁盘映像必须已挂载");
        process::run(
            Command::new("hdiutil").arg("detach").arg(mountpoint.path()),
            &format!("卸载 {}", mountpoint.path().display()),
        )?;
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
