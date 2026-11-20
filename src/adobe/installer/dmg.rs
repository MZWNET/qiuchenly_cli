use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

use super::super::model::DownloadedProduct;
use super::super::system;

pub(super) fn install_dmg_package(product: &DownloadedProduct) -> Result<()> {
    let mut dmgs = Vec::new();
    for downloaded in &product.packages {
        if downloaded
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dmg"))
        {
            dmgs.push(downloaded);
        }
    }
    if dmgs.is_empty() {
        bail!("DMG 产品未提供 DMG 包");
    }
    if dmgs.len() != 1 {
        bail!("产品包含多个 DMG 包，无法确定安装介质");
    }
    let downloaded = dmgs.remove(0);
    let mut image = crate::macos::archive::image::MountedImage::attach(&downloaded.path)
        .with_context(|| format!("挂载 {}", downloaded.path.display()))?;
    let result = install_pkgs(image.path());
    let detach = image.detach();
    result?;
    detach.context("卸载 DMG")?;
    Ok(())
}

fn install_pkgs(root: &Path) -> Result<()> {
    let mut pkgs = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry.with_context(|| format!("遍历 {}", root.display()))?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pkg"))
        {
            pkgs.push(entry.into_path());
        }
    }
    if pkgs.is_empty() {
        bail!("DMG 中未找到 .pkg");
    }
    for pkg in pkgs {
        let choices = arma_deselected_choices(&pkg)?;
        let choices_file = if choices.is_empty() {
            None
        } else {
            let xml = build_choices_xml(&choices);
            // 绑定变量使暂存目录存活到 installer 运行结束
            let staging = tempfile::tempdir().context("创建 choices 暂存目录")?;
            let path = staging.path().join("choices.xml");
            fs::write(&path, xml).context("写入 choices XML")?;
            Some((staging, path))
        };
        let mut command = system::privileged_command("/usr/sbin/installer");
        command
            .arg("-pkg")
            .arg(&pkg)
            .arg("-target")
            .arg("/")
            .arg("-verboseR");
        if let Some((_, path)) = &choices_file {
            command.arg("-applyChoiceChangesXML").arg(path);
        }
        let output = command
            .output()
            .with_context(|| format!("运行 installer {}", pkg.display()))?;
        if !output.status.success() {
            bail!(
                "安装 {} 失败: {}",
                pkg.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(())
}

fn extract_choice_identifiers(choices_xml: &str) -> Vec<String> {
    let Ok(document) = roxmltree::Document::parse(choices_xml) else {
        return Vec::new();
    };
    let mut identifiers = Vec::new();
    for dict in document
        .descendants()
        .filter(|node| node.tag_name().name() == "dict")
    {
        let mut key = None;
        for child in dict.children().filter(|node| node.is_element()) {
            match child.tag_name().name() {
                "key" => key = child.text(),
                "string" => {
                    if key.is_some_and(|key| key == "choiceIdentifier") {
                        if let Some(text) = child.text() {
                            identifiers.push(text.to_owned());
                        }
                    }
                    key = None;
                }
                _ => {}
            }
        }
    }
    identifiers
}

// ARMAgent 是 Adobe 后台遥测代理，安装时取消勾选
fn arma_deselected_choices(pkg: &Path) -> Result<Vec<String>> {
    let output = system::privileged_command("/usr/sbin/installer")
        .args(["-showChoicesXML", "-pkg"])
        .arg(pkg)
        .arg("-target")
        .arg("/")
        .output()
        .with_context(|| format!("查询 {} 的安装选项", pkg.display()))?;
    if !output.status.success() {
        bail!(
            "查询 {} 安装选项失败: {}",
            pkg.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let xml = String::from_utf8_lossy(&output.stdout).into_owned();
    Ok(extract_choice_identifiers(&xml)
        .into_iter()
        .filter(|identifier| identifier.contains("ARMAgent"))
        .collect())
}

fn build_choices_xml(identifiers: &[String]) -> String {
    let choices: Vec<plist::Dictionary> = identifiers
        .iter()
        .map(|identifier| {
            let mut dict = plist::Dictionary::new();
            dict.insert(
                "choiceAttribute".to_owned(),
                plist::Value::String("selected".to_owned()),
            );
            dict.insert(
                "choiceIdentifier".to_owned(),
                plist::Value::String(identifier.clone()),
            );
            dict.insert(
                "attributeSetting".to_owned(),
                plist::Value::Integer(0.into()),
            );
            dict
        })
        .collect();
    let value = plist::Value::Array(choices.into_iter().map(plist::Value::Dictionary).collect());
    let mut buffer = Vec::new();
    plist::to_writer_xml(&mut buffer, &value).expect("plist 序列化不会失败");
    String::from_utf8(buffer).expect("plist XML 输出必须是 UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_arma_choice_identifiers() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><array>
<dict><key>choiceIdentifier</key><string>com.adobe.armsvc</string><key>choiceAttribute</key><string>selected</string></dict>
<dict><key>choiceIdentifier</key><string>com.adobe.ARMAgent</string></dict>
<dict><key>otherKey</key><string>not-a-choice</string></dict>
</array></plist>"#;
        let identifiers = extract_choice_identifiers(xml);
        assert_eq!(identifiers, vec!["com.adobe.armsvc", "com.adobe.ARMAgent"]);
        assert!(identifiers
            .iter()
            .any(|identifier| identifier.contains("ARMAgent")));
    }

    #[test]
    fn builds_choices_xml_with_deselection() {
        let xml = build_choices_xml(&["com.adobe.ARMA&gent<x>".to_owned()]);
        assert!(xml.contains("<string>com.adobe.ARMA&amp;gent&lt;x&gt;</string>"));
        assert!(xml.contains("<integer>0</integer>"));
    }
}
