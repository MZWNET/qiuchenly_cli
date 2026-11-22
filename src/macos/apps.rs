use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::{DirEntry, WalkDir};

pub struct InstalledApp {
    pub bundle_id: String,
    pub name: String,
    pub version: String,
    pub path: PathBuf,
}

pub fn parse_app_bundle(path: &Path) -> Result<InstalledApp> {
    let plist_path = path.join("Contents/Info.plist");
    // 破解版 plist 常含重复键, serde 结构体会报错, Value 取后值
    let info = plist::Value::from_file(&plist_path)
        .with_context(|| format!("读取 {}", plist_path.display()))?;
    let dict = info
        .as_dictionary()
        .with_context(|| format!("{} 不是字典结构", plist_path.display()))?;
    let field = |key: &str| {
        dict.get(key)
            .and_then(plist::Value::as_string)
            .map(|s| s.trim().to_owned())
    };
    let bundle_id = field("CFBundleIdentifier").unwrap_or_default();
    if bundle_id.is_empty() {
        bail!("{} 缺少应用标识", plist_path.display());
    }

    Ok(InstalledApp {
        bundle_id,
        name: field("CFBundleName")
            .or_else(|| field("CFBundleDisplayName"))
            .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default(),
        version: field("CFBundleShortVersionString").unwrap_or_default(),
        path: path.to_path_buf(),
    })
}

fn is_app(entry: &DirEntry) -> bool {
    entry.file_type().is_dir() && entry.path().extension().is_some_and(|ext| ext == "app")
}

pub fn scan_installed_apps() -> Result<Vec<InstalledApp>> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    let mut apps = Vec::new();
    for root in roots.into_iter().filter(|root| root.is_dir()) {
        let mut entries = WalkDir::new(root)
            .min_depth(1)
            .max_depth(2)
            .follow_links(false)
            .follow_root_links(false)
            .sort_by_file_name()
            .into_iter();
        while let Some(entry) = entries.next() {
            let entry = entry?;
            if is_app(&entry) {
                entries.skip_current_dir();
                if let Ok(app) = parse_app_bundle(entry.path()) {
                    apps.push(app);
                }
            }
        }
    }
    apps.sort_by(|a, b| {
        a.bundle_id
            .cmp(&b.bundle_id)
            .then_with(|| a.path.cmp(&b.path))
    });
    apps.dedup_by(|a, b| a.bundle_id == b.bundle_id);

    Ok(apps)
}

pub fn find_installed_app(bundle_id: &str) -> Result<InstalledApp> {
    scan_installed_apps()?
        .into_iter()
        .find(|app| app.bundle_id == bundle_id)
        .ok_or_else(|| anyhow::anyhow!("本地未安装 {bundle_id}"))
}

pub fn clear_extended_attributes(path: &Path) -> Result<()> {
    let output = Command::new("/usr/bin/xattr")
        .arg("-cr")
        .arg(path)
        .output()
        .context("运行 xattr 清理扩展属性")?;
    if !output.status.success() {
        bail!(
            "清理 {} 扩展属性失败: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_app_bundle;
    use plist::{Dictionary, Value};
    use std::fs;

    #[test]
    fn parses_binary_plist_and_requires_bundle_id() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Example.app");
        let contents = app.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        let path = contents.join("Info.plist");
        let mut info = Dictionary::new();
        info.insert(
            "CFBundleIdentifier".into(),
            Value::String("org.example.app".into()),
        );
        info.insert(
            "CFBundleDisplayName".into(),
            Value::String("Example".into()),
        );
        info.insert(
            "CFBundleShortVersionString".into(),
            Value::String("1.2".into()),
        );
        plist::to_file_binary(&path, &info).unwrap();
        let parsed = parse_app_bundle(&app).unwrap();
        assert_eq!(parsed.bundle_id, "org.example.app");
        assert_eq!(parsed.name, "Example");
        assert_eq!(parsed.version, "1.2");
        info.remove("CFBundleIdentifier");
        plist::to_file_binary(&path, &info).unwrap();
        assert!(parse_app_bundle(&app).is_err());
        info.insert("CFBundleIdentifier".into(), Value::String(String::new()));
        plist::to_file_binary(&path, &info).unwrap();
        assert!(parse_app_bundle(&app).is_err());
    }

    #[test]
    fn tolerates_duplicate_plist_keys() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Example.app");
        let contents = app.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(
            contents.join("Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.parallels.desktop.console</string>
    <key>CFBundleShortVersionString</key>
    <string>26.4.1</string>
    <key>CFBundleShortVersionString</key>
    <string>26.4.1</string>
</dict>
</plist>"#,
        )
        .unwrap();
        let parsed = parse_app_bundle(&app).unwrap();
        assert_eq!(parsed.bundle_id, "com.parallels.desktop.console");
        assert_eq!(parsed.version, "26.4.1");
    }
}
