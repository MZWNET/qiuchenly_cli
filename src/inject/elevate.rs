use anyhow::{bail, Context, Result};
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::process::Command;

pub(super) fn is_root() -> bool {
    // SAFETY: geteuid 无参数、无副作用
    (unsafe { libc::geteuid() }) == 0
}

pub(super) fn is_permission_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == ErrorKind::PermissionDenied)
    })
}

fn shell_quote(value: &OsStr) -> String {
    let escaped = value.to_string_lossy().replace('\'', r"'\''");
    format!("'{escaped}'")
}

pub(super) fn run_elevated() -> Result<()> {
    if is_root() {
        bail!("提权重试时已是 root");
    }
    let exe = std::env::current_exe().context("获取当前可执行文件路径失败")?;
    let cwd = std::env::current_dir().context("获取当前目录失败")?;
    let mut command = format!("cd {} && ", shell_quote(cwd.as_os_str()));
    for key in ["HAYAKU_CONFIG", "HAYAKU_INJECT_DYLIB"] {
        if let Some(value) = std::env::var_os(key) {
            command.push_str(&format!("{key}={} ", shell_quote(&value)));
        }
    }
    command.push_str(&shell_quote(exe.as_os_str()));
    for arg in std::env::args_os().skip(1) {
        command.push(' ');
        command.push_str(&shell_quote(&arg));
    }
    println!("请求管理员权限...");
    let output = Command::new("osascript")
        .arg("-e")
        .arg("on run argv")
        .arg("-e")
        .arg("do shell script (item 1 of argv) with administrator privileges")
        .arg("--")
        .arg(&command)
        .output()
        .context("运行 osascript 请求管理员权限失败")?;
    if !output.status.success() {
        bail!(
            "提权执行失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));

    Ok(())
}
