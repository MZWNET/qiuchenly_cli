use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

pub(crate) fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub(crate) fn privileged_command(program: &str) -> Command {
    if is_root() {
        Command::new(program)
    } else {
        let mut command = Command::new("sudo");
        command.arg(program);
        command
    }
}

pub(crate) fn is_system_path(path: &Path) -> bool {
    path.starts_with("/Library") || path.starts_with("/Applications")
}

pub(crate) fn run_command(command: &mut Command, description: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("运行 {description}"))?;
    if !output.status.success() {
        bail!(
            "{description}失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(crate) fn install_file(source: &Path, target: &Path) -> Result<()> {
    ensure_parent(target)?;
    if is_system_path(target) {
        run_command(
            privileged_command("ditto").arg(source).arg(target),
            &format!("复制 {} 到 {}", source.display(), target.display()),
        )
    } else {
        fs::copy(source, target)
            .with_context(|| format!("复制 {} 到 {}", source.display(), target.display()))?;
        Ok(())
    }
}

fn ensure_parent(target: &Path) -> Result<()> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    if is_system_path(parent) {
        run_command(
            privileged_command("mkdir").arg("-p").arg(parent),
            &format!("创建目录 {}", parent.display()),
        )
    } else {
        fs::create_dir_all(parent).with_context(|| format!("创建目录 {}", parent.display()))
    }
}

pub(crate) fn copy_asset(source: &Path, target: &Path, recursive: bool) -> Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("警告: PIMX 资源源路径不存在，跳过: {}", source.display());
            return Ok(());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("读取 {}", source.display()));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        copy_symlink(source, target)
    } else if file_type.is_dir() {
        if !recursive {
            bail!("目录资产未标记 recursive: {}", source.display());
        }
        copy_tree(source, target)
    } else {
        copy_file(source, target)
    }
}

fn copy_file(source: &Path, target: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(target) {
        let file_type = metadata.file_type();
        if file_type.is_dir() || file_type.is_symlink() {
            remove_path(target)?;
        }
    }
    install_file(source, target)
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(target) {
        let file_type = metadata.file_type();
        if file_type.is_file() || file_type.is_symlink() {
            remove_path(target)?;
        }
    }
    if is_system_path(target) {
        ensure_parent(target)?;
        run_command(
            privileged_command("ditto").arg(source).arg(target),
            &format!("复制 {} 到 {}", source.display(), target.display()),
        )
    } else {
        copy_tree_unprivileged(source, target)
    }
}

fn copy_tree_unprivileged(source: &Path, target: &Path) -> Result<()> {
    let mut dir_modes = Vec::new();
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry.with_context(|| format!("遍历 {}", source.display()))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .expect("遍历路径必须位于源目录内");
        let destination = target.join(relative);
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            let pointee = fs::read_link(entry.path())
                .with_context(|| format!("读取符号链接 {}", entry.path().display()))?;
            create_symlink(&pointee, &destination)?;
        } else if file_type.is_dir() {
            fs::create_dir_all(&destination)
                .with_context(|| format!("创建目录 {}", destination.display()))?;
            let mode = fs::metadata(entry.path())
                .with_context(|| format!("读取 {}", entry.path().display()))?
                .permissions()
                .mode();
            dir_modes.push((destination, mode));
        } else {
            fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "复制 {} 到 {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        }
    }
    // 目录权限最后反向应用，避免只读目录阻断子项写入
    for (directory, mode) in dir_modes.iter().rev() {
        fs::set_permissions(directory, fs::Permissions::from_mode(*mode))
            .with_context(|| format!("设置权限 {}", directory.display()))?;
    }
    Ok(())
}

fn copy_symlink(source: &Path, target: &Path) -> Result<()> {
    let pointee =
        fs::read_link(source).with_context(|| format!("读取符号链接 {}", source.display()))?;
    create_symlink(&pointee, target)
}

pub(crate) fn create_symlink(pointee: &Path, link: &Path) -> Result<()> {
    if let Ok(existing) = fs::read_link(link) {
        if existing == pointee {
            return Ok(());
        }
    }
    if fs::symlink_metadata(link).is_ok() {
        remove_path(link)?;
    }
    ensure_parent(link)?;
    if is_system_path(link) {
        run_command(
            privileged_command("ln").arg("-s").arg(pointee).arg(link),
            &format!("创建符号链接 {}", link.display()),
        )
    } else {
        symlink(pointee, link)
            .with_context(|| format!("创建符号链接 {} -> {}", link.display(), pointee.display()))
    }
}

pub(crate) fn remove_path(target: &Path) -> Result<()> {
    if is_system_path(target) {
        run_command(
            privileged_command("rm").arg("-rf").arg(target),
            &format!("删除 {}", target.display()),
        )
    } else {
        let file_type = fs::symlink_metadata(target)
            .with_context(|| format!("读取 {}", target.display()))?
            .file_type();
        if file_type.is_symlink() || file_type.is_file() {
            fs::remove_file(target).with_context(|| format!("删除 {}", target.display()))
        } else {
            fs::remove_dir_all(target).with_context(|| format!("删除目录 {}", target.display()))
        }
    }
}

pub(crate) fn create_directory(path: &Path) -> Result<()> {
    if is_system_path(path) {
        run_command(
            privileged_command("mkdir").arg("-p").arg(path),
            &format!("创建目录 {}", path.display()),
        )
    } else {
        fs::create_dir_all(path).with_context(|| format!("创建目录 {}", path.display()))
    }
}

pub(crate) fn create_user_pref(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建目录 {}", parent.display()))?;
    }
    if !path.exists() {
        fs::File::create(path).with_context(|| format!("创建用户偏好文件 {}", path.display()))?;
    }
    Ok(())
}
