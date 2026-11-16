use super::config::AppConfig;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::Command;

const ENTITLEMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>com.apple.security.cs.allow-jits</key><true/>
	<key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
	<key>com.apple.security.cs.disable-library-validation</key><true/>
	<key>com.apple.security.get-task-allow</key><true/>
</dict>
</plist>
"#;

pub(super) fn codesign(target: &Path, deep: bool, with_entitlements: bool) -> Result<()> {
    let mut command = Command::new("codesign");
    command.args(["--force", "--sign", "-"]);
    if deep {
        command.arg("--deep");
    }
    let entitlements = if with_entitlements {
        let mut file = tempfile::NamedTempFile::new().context("创建注入签名权限文件")?;
        file.write_all(ENTITLEMENTS.as_bytes())
            .context("写入注入签名权限")?;
        command.arg("--entitlements").arg(file.path());
        Some(file)
    } else {
        None
    };
    let status = command
        .arg(target)
        .status()
        .with_context(|| format!("运行 codesign 重签 {}", target.display()))?;
    drop(entitlements);
    if !status.success() {
        bail!("codesign 重签 {} 失败: {status}", target.display());
    }

    Ok(())
}

pub(super) fn reset_tcc(config: &AppConfig, bundle_id: &str) -> Result<()> {
    match &config.tccutil {
        Some(serde_json::Value::Bool(true)) => reset_service("All", bundle_id)?,
        Some(serde_json::Value::Array(services)) => {
            for service in services.iter().filter_map(serde_json::Value::as_str) {
                reset_service(service, bundle_id)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn reset_service(service: &str, bundle_id: &str) -> Result<()> {
    let status = Command::new("tccutil")
        .args(["reset", service, bundle_id])
        .status()
        .with_context(|| format!("运行 tccutil reset {service} {bundle_id}"))?;
    if !status.success() {
        bail!("tccutil reset {service} {bundle_id} 失败: {status}");
    }

    Ok(())
}
