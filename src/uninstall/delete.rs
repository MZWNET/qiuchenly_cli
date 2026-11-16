use super::scan::{Identity, ScanItem};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn delete(item: &ScanItem, protected: &[PathBuf]) -> Result<()> {
    let path = &item.path;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!("无效的删除路径: {}", path.display());
    }
    for blocked in protected
        .iter()
        .map(PathBuf::as_path)
        .chain([Path::new("/System"), Path::new("/private/var/db")])
    {
        if path.starts_with(blocked) || blocked.starts_with(path) {
            bail!("保护路径: {}", path.display());
        }
    }
    let mut parent = PathBuf::new();
    for component in path.parent().context("删除路径没有父目录")?.components() {
        parent.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&parent)
            .with_context(|| format!("检查父目录 {} 失败", parent.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("父目录包含符号链接: {}", parent.display());
        }
    }
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("检查 {} 失败", path.display()))?;
    if Identity::from_metadata(&metadata) != item.identity {
        bail!("扫描后目标已改变: {}", path.display());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn rejects_replaced_candidate_and_parent_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let parent = root.join("parent");
        let outside = root.join("outside");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        let path = parent.join("item");
        fs::write(&path, "original").unwrap();
        let item = ScanItem::app(path.clone()).unwrap();
        fs::rename(&path, parent.join("original")).unwrap();
        fs::write(&path, "replacement").unwrap();
        assert!(delete(&item, &[]).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        fs::write(outside.join("item"), "outside").unwrap();
        fs::rename(&parent, root.join("old-parent")).unwrap();
        symlink(&outside, &parent).unwrap();
        assert!(delete(&item, &[]).is_err());
        assert_eq!(fs::read_to_string(outside.join("item")).unwrap(), "outside");
    }

    #[test]
    fn blocks_protected_paths_but_deletes_symlink_itself() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let protected = root.join("protected");
        fs::create_dir(&protected).unwrap();
        let target = protected.join("data");
        fs::write(&target, "safe").unwrap();
        let item = ScanItem::app(target.clone()).unwrap();
        assert!(delete(&item, std::slice::from_ref(&protected)).is_err());
        let link = root.join("link");
        symlink(&target, &link).unwrap();
        delete(&ScanItem::app(link.clone()).unwrap(), &[protected]).unwrap();
        assert!(fs::symlink_metadata(link).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "safe");
    }
}
