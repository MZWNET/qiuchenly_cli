use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub(super) fn publish(stage: TempDir, staged: &Path, destination: &Path) -> Result<PathBuf> {
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
