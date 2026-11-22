use super::system::{
    self, copy_asset, create_symlink, is_system_path, privileged_command, remove_path,
};
use crate::macos::process;
use anyhow::{bail, Context, Result};
use roxmltree::Node;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

pub(crate) struct InstallContext {
    pub(crate) staging_folder: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) install_dir: PathBuf,
    pub(crate) base_version: String,
    pub(crate) language: String,
    pub(crate) architecture: String,
    pub(crate) home: PathBuf,
}

pub(crate) fn lzma2_dict_size(prop: u8) -> u64 {
    ((prop as u64 & 1) | 2) << ((prop as u64 >> 1) + 11)
}

pub(crate) fn decode_pimx(bytes: &[u8]) -> Result<String> {
    let body = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if body.first() == Some(&b'<') {
        return String::from_utf8(body.to_vec()).context("PIMX 不是有效的 UTF-8 文本");
    }
    let Some((prop, compressed)) = body.split_first() else {
        bail!("PIMX 文件为空");
    };
    if lzma2_dict_size(*prop) > 1 << 32 {
        bail!("PIMX 属性字节 {prop} 无效，字典大小超过 4GiB");
    }
    let mut output = Vec::new();
    lzma_rs::lzma2_decompress(&mut std::io::Cursor::new(compressed), &mut output)
        .context("解压 PIMX LZMA2 数据")?;
    String::from_utf8(output).context("解压后的 PIMX 不是有效的 UTF-8 文本")
}

impl InstallContext {
    pub(crate) fn new(
        template: &str,
        destination: &Path,
        home: &Path,
        staging_folder: &Path,
        base_version: &str,
        language: &str,
        architecture: &str,
    ) -> Result<Self> {
        let mut context = Self {
            staging_folder: destination.to_path_buf(),
            destination: destination.to_path_buf(),
            install_dir: destination.to_path_buf(),
            base_version: String::new(),
            language: String::new(),
            architecture: String::new(),
            home: home.to_path_buf(),
        };
        context.install_dir = PathBuf::from(context.expand(template)?);
        context.staging_folder = staging_folder.to_path_buf();
        context.base_version = base_version.to_owned();
        context.language = language.to_owned();
        context.architecture = architecture.to_owned();
        Ok(context)
    }

    pub(crate) fn expand(&self, text: &str) -> Result<String> {
        let home = self.home.display().to_string();
        let replacements = [
            ("[StagingFolder]", self.staging_folder.display().to_string()),
            ("[INSTALLDIR]", self.install_dir.display().to_string()),
            ("[InstallDir]", self.install_dir.display().to_string()),
            (
                "[AdobeProgramFiles]",
                self.destination.display().to_string(),
            ),
            (
                "[AdobeCommon]",
                "/Library/Application Support/Adobe".to_owned(),
            ),
            (
                "[GLOBAL_SUPPORT]",
                "/Library/Application Support".to_owned(),
            ),
            (
                "[SharedApplicationData]",
                "/Library/Application Support".to_owned(),
            ),
            ("[UserPreferences]", format!("{home}/Library/Preferences")),
            (
                "[UserCommon]",
                format!("{home}/Library/Application Support"),
            ),
            ("[UserLibrary]", format!("{home}/Library")),
            ("[USER]", home.clone()),
            ("[UserHome]", home.clone()),
            ("[UserDocuments]", format!("{home}/Documents")),
            ("[SharedDocuments]", "/Users/Shared".to_owned()),
            ("[installLanguage]", self.language.clone()),
            ("[OSArchitecture]", self.architecture.clone()),
            ("$APPBASEVERSION", self.base_version.clone()),
            ("[prefMigration]", "1".to_owned()),
        ];
        let mut expanded = text.to_owned();
        for (token, value) in replacements {
            expanded = expanded.replace(token, &value);
        }
        if let Some(unknown) = unknown_placeholder(&expanded) {
            bail!("不支持的占位符: {unknown}");
        }
        Ok(expanded)
    }
}

fn unknown_placeholder(text: &str) -> Option<String> {
    let mut start = None;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'[' => start = Some(index),
            b']' => {
                if let Some(begin) = start {
                    let token = &text[begin..=index];
                    if !token.contains('/') {
                        return Some(token.to_owned());
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn install_pimx(xml: &str, context: &InstallContext) -> Result<Option<PathBuf>> {
    let document = roxmltree::Document::parse(xml).context("解析 PIMX XML")?;
    let root = document.root_element();
    if root.tag_name().name() != "Package" {
        bail!("PIMX 根节点不是 Package: {}", root.tag_name().name());
    }
    let mut application_xml = None;
    for section in root.children().filter(Node::is_element) {
        match section.tag_name().name() {
            "Assets" => {
                if let Some(xml) = install_assets(&section, context)? {
                    application_xml = Some(xml);
                }
            }
            "Commands" => run_commands(&section, context)?,
            // Condition 已在下载阶段按包过滤，无需重复判断
            _ => {}
        }
    }
    Ok(application_xml)
}

fn install_assets(assets: &Node, context: &InstallContext) -> Result<Option<PathBuf>> {
    let mut application_xml = None;
    for asset in assets.children().filter(Node::is_element) {
        if asset.tag_name().name() != "Asset" {
            bail!("Assets 中出现未知节点: {}", asset.tag_name().name());
        }
        if asset
            .attribute("ignoreAsset")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        {
            continue;
        }
        let source = asset
            .attribute("source")
            .context("Asset 缺少 source 属性")?;
        let target = asset
            .attribute("target")
            .context("Asset 缺少 target 属性")?;
        let recursive = asset
            .attribute("recursive")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let source = context.expand(source)?;
        let target = validate_target(&context.expand(target)?)?;
        copy_asset(Path::new(&source), &target, recursive)
            .with_context(|| format!("安装资产 {source} 到 {}", target.display()))?;
        if let Some(xml) = application_xml_path(&target) {
            application_xml = Some(xml);
        }
    }
    Ok(application_xml)
}

fn application_xml_path(target: &Path) -> Option<PathBuf> {
    let in_amt = target
        .parent()
        .is_some_and(|parent| parent.file_name().is_some_and(|name| name == "AMT"));
    if target.is_dir() {
        if target.file_name().is_some_and(|name| name == "AMT") {
            let xml = target.join("application.xml");
            if xml.is_file() {
                return Some(xml);
            }
        }
    } else if in_amt
        && target
            .file_name()
            .is_some_and(|name| name == "application.xml")
    {
        return Some(target.to_path_buf());
    }
    None
}

fn validate_target(target: &str) -> Result<PathBuf> {
    let path = PathBuf::from(target);
    if !path.is_absolute() {
        bail!("PIMX 目标路径不是绝对路径: {target}");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("PIMX 目标路径包含目录穿越: {target}");
    }
    Ok(path)
}

fn child_text<'a>(node: &Node<'a, 'a>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| child.text())
}

fn expand_path(node: &Node, context: &InstallContext, tag: &str) -> Result<PathBuf> {
    let text = child_text(node, tag)
        .with_context(|| format!("PIMX 命令 {} 缺少 {tag}", node.tag_name().name()))?;
    Ok(PathBuf::from(context.expand(text.trim())?))
}

fn run_commands(commands: &Node, context: &InstallContext) -> Result<()> {
    for command in commands.children().filter(Node::is_element) {
        match command.tag_name().name() {
            "FolderIcon" => {} // 仅影响文件夹外观
            "Permission" => run_permission(&command, context)?,
            "Owner" => run_owner(&command, context)?,
            "RegisterApplication" => register_application(&command, context)?,
            "InstallUserPref" => install_user_pref(&command, context)?,
            "RunProgram" => run_program(&command, context)?,
            "DeleteFile" => delete_file(&command, context)?,
            "DeleteDirectory" => delete_directory(&command, context)?,
            "CreateDirectory" | "CreateFolder" => create_directory(&command, context)?,
            "CreateSymlink" => create_symlink_command(&command, context)?,
            other => bail!("不支持的 PIMX 命令: {other}"),
        }
    }
    Ok(())
}

fn run_permission(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    let value = child_text(command, "PermissionValue")
        .context("Permission 缺少 PermissionValue")?
        .trim();
    let mut chmod = if is_system_path(&path) {
        privileged_command("chmod")
    } else {
        Command::new("chmod")
    };
    process::run(
        chmod.args(["-R", value]).arg(&path),
        &format!("设置 {} 权限为 {value}", path.display()),
    )
}

fn run_owner(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    let user = child_text(command, "User")
        .context("Owner 缺少 User")?
        .trim();
    let group = child_text(command, "Group")
        .context("Owner 缺少 Group")?
        .trim();
    process::run(
        privileged_command("chown")
            .arg("-R")
            .arg(format!("{user}:{group}"))
            .arg(&path),
        &format!("设置 {} 所有者为 {user}:{group}", path.display()),
    )
}

fn register_application(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    if !Path::new(LSREGISTER).exists() {
        bail!("lsregister 工具不存在");
    }
    process::run(
        Command::new(LSREGISTER).arg("-f").arg(&path),
        &format!("注册应用 {}", path.display()),
    )
}

fn install_user_pref(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    system::create_user_pref(&path)
}

fn run_program(command: &Node, context: &InstallContext) -> Result<()> {
    let install = command
        .children()
        .find(|child| child.has_tag_name("InstallCommand"))
        .context("RunProgram 缺少 InstallCommand")?;
    let user_mode = install
        .attribute("runInUserMode")
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));
    let program = child_text(&install, "Path").context("InstallCommand 缺少 Path")?;
    let program = context.expand(program.trim())?;
    let mut arguments = Vec::new();
    if let Some(nodes) = install
        .children()
        .find(|child| child.has_tag_name("Arguments"))
    {
        for argument in nodes
            .children()
            .filter(|child| child.has_tag_name("Argument"))
        {
            arguments.push(context.expand(argument.text().unwrap_or_default().trim())?);
        }
    }
    let mut success_codes = vec![0];
    if let Some(nodes) = install
        .children()
        .find(|child| child.has_tag_name("SuccessExitCodes"))
    {
        success_codes = nodes
            .children()
            .filter(|child| child.has_tag_name("ExitCode"))
            .map(|code| code.text().unwrap_or_default().trim().parse::<i32>())
            .collect::<Result<Vec<i32>, _>>()
            .context("解析 SuccessExitCodes")?;
    }
    let mut process = if user_mode {
        Command::new(&program)
    } else {
        privileged_command(&program)
    };
    process.args(&arguments);
    let status = process
        .status()
        .with_context(|| format!("运行 {program}"))?;
    let code = status.code().unwrap_or(-1);
    if !success_codes.contains(&code) {
        bail!("运行 {program} 退出码 {code}，不在期望值 {success_codes:?} 中");
    }
    Ok(())
}

fn delete_file(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if !metadata.file_type().is_dir() {
            remove_path(&path)?;
        }
    }
    Ok(())
}

fn delete_directory(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    if fs::symlink_metadata(&path).is_ok() {
        remove_path(&path)?;
    }
    Ok(())
}

fn create_directory(command: &Node, context: &InstallContext) -> Result<()> {
    let path = expand_path(command, context, "Path")?;
    system::create_directory(&path)
}

fn create_symlink_command(command: &Node, context: &InstallContext) -> Result<()> {
    let pointee = expand_path(command, context, "Target")?;
    let link = expand_path(command, context, "Path")?;
    create_symlink(&pointee, &link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn test_context(staging: &Path, home: &Path) -> InstallContext {
        InstallContext::new(
            "[AdobeProgramFiles]/Adobe Test",
            &home.join("Applications"),
            home,
            staging,
            "1.0",
            "en_US",
            "arm64",
        )
        .unwrap()
    }

    #[test]
    fn computes_lzma2_dict_size() {
        assert_eq!(lzma2_dict_size(0), 4096);
        assert_eq!(lzma2_dict_size(24), 16 * 1024 * 1024);
        assert_eq!(lzma2_dict_size(40), 1 << 32);
        assert!(lzma2_dict_size(41) > 1 << 32);
    }

    #[test]
    fn resolves_install_dir_placeholders() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            InstallContext::new(
                "[AdobeProgramFiles]/Adobe Photoshop 2027",
                Path::new("/Applications"),
                home.path(),
                home.path(),
                "1.0",
                "en_US",
                "arm64"
            )
            .unwrap()
            .install_dir,
            PathBuf::from("/Applications/Adobe Photoshop 2027")
        );
        assert_eq!(
            InstallContext::new(
                "[AdobeCommon]/Shared",
                Path::new("/Applications"),
                home.path(),
                home.path(),
                "1.0",
                "en_US",
                "arm64"
            )
            .unwrap()
            .install_dir,
            PathBuf::from("/Library/Application Support/Adobe/Shared")
        );
    }

    #[test]
    fn initializes_context_before_resolving_install_dir() {
        let home = tempfile::tempdir().unwrap();
        let destination = home.path().join("Applications");
        let staging = home.path().join("staging");
        let context = InstallContext::new(
            "[StagingFolder]/[installLanguage]$APPBASEVERSION[OSArchitecture]",
            &destination,
            home.path(),
            &staging,
            "1.0",
            "en_US",
            "arm64",
        )
        .unwrap();
        assert_eq!(context.install_dir, destination.join(""));
        assert_eq!(
            context
                .expand("[StagingFolder]/[installLanguage]/$APPBASEVERSION/[OSArchitecture]")
                .unwrap(),
            format!("{}/en_US/1.0/arm64", staging.display())
        );
        assert!(InstallContext::new(
            "[Unknown]",
            &destination,
            home.path(),
            &staging,
            "1.0",
            "en_US",
            "arm64"
        )
        .is_err());
    }

    #[test]
    fn decodes_plain_and_lzma2_pimx() {
        let xml = r#"<?xml version="1.0"?><Package><PackageName>Test</PackageName></Package>"#;
        assert_eq!(decode_pimx(xml.as_bytes()).unwrap(), xml);
        let mut compressed = Vec::new();
        lzma_rs::lzma2_compress(&mut xml.as_bytes(), &mut compressed).unwrap();
        let mut bytes = vec![24u8];
        bytes.extend_from_slice(&compressed);
        assert_eq!(decode_pimx(&bytes).unwrap(), xml);
        assert!(decode_pimx(&[41, 0]).is_err());
    }

    #[test]
    fn expands_known_placeholders_and_rejects_unknown() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let context = test_context(staging.path(), home.path());
        assert_eq!(context.expand("[installLanguage]").unwrap(), "en_US");
        assert_eq!(context.expand("$APPBASEVERSION").unwrap(), "1.0");
        assert_eq!(
            context.expand("[AdobeCommon]/AMT").unwrap(),
            "/Library/Application Support/Adobe/AMT"
        );
        assert_eq!(
            context.expand("[UserPreferences]/x").unwrap(),
            home.path()
                .join("Library/Preferences/x")
                .display()
                .to_string()
        );
        assert!(context.expand("[Bogus]").is_err());
        assert!(context.expand("[foo]bar").is_err());
    }

    #[test]
    fn installs_assets_from_plain_xml() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        fs::write(staging.path().join("payload.txt"), "data").unwrap();
        fs::create_dir_all(staging.path().join("Settings")).unwrap();
        fs::write(staging.path().join("Settings").join("theme.txt"), "theme").unwrap();
        let xml = r#"<Package><Assets>
            <Asset source="[StagingFolder]/payload.txt" target="[UserPreferences]/payload.txt"/>
            <Asset source="[StagingFolder]/Settings" target="[UserCommon]/Adobe/Test/Settings" recursive="true"/>
            <Asset source="[StagingFolder]/missing.txt" target="[UserPreferences]/missing.txt"/>
            <Asset source="[StagingFolder]/payload.txt" target="[UserPreferences]/ignored.txt" ignoreAsset="true"/>
        </Assets></Package>"#;
        let context = test_context(staging.path(), home.path());
        let tracked = install_pimx(xml, &context).unwrap();
        let preferences = home.path().join("Library/Preferences");
        assert_eq!(
            fs::read_to_string(preferences.join("payload.txt")).unwrap(),
            "data"
        );
        assert_eq!(
            fs::read_to_string(
                home.path()
                    .join("Library/Application Support/Adobe/Test/Settings/theme.txt")
            )
            .unwrap(),
            "theme"
        );
        assert!(!preferences.join("missing.txt").exists());
        assert!(!preferences.join("ignored.txt").exists());
        assert!(tracked.is_none());
    }

    #[test]
    fn tracks_application_xml_in_amt_directory() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let amt = staging.path().join("AMT");
        fs::create_dir_all(&amt).unwrap();
        fs::write(amt.join("application.xml"), "<Configuration/>").unwrap();
        let xml = r#"<Package><Assets>
            <Asset source="[StagingFolder]/AMT" target="[UserCommon]/Adobe/Test/AMT" recursive="true"/>
            <Asset source="[StagingFolder]/AMT/application.xml" target="[UserCommon]/Adobe/Test/Alt/AMT/application.xml"/>
        </Assets></Package>"#;
        let context = test_context(staging.path(), home.path());
        let mut tracked = install_pimx(xml, &context).unwrap();
        let base = home.path().join("Library/Application Support/Adobe/Test");
        assert_eq!(
            tracked,
            Some(base.join("Alt").join("AMT").join("application.xml"))
        );
        assert_eq!(install_pimx("<Package/>", &context).unwrap(), None);
        assert_eq!(
            tracked,
            Some(base.join("Alt").join("AMT").join("application.xml"))
        );
        let next_xml = r#"<Package><Assets><Asset source="[StagingFolder]/AMT/application.xml" target="[UserCommon]/Adobe/Test/Last/AMT/application.xml"/></Assets></Package>"#;
        if let Some(xml) = install_pimx(next_xml, &context).unwrap() {
            tracked = Some(xml);
        }
        assert_eq!(
            tracked,
            Some(base.join("Last").join("AMT").join("application.xml"))
        );
    }

    #[test]
    fn preserves_symlink_and_executable_mode() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let bundle = staging.path().join("Bundle");
        fs::create_dir_all(&bundle).unwrap();
        let bin = bundle.join("bin.sh");
        fs::write(&bin, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        symlink("bin.sh", bundle.join("link")).unwrap();
        let xml = r#"<Package><Assets>
            <Asset source="[StagingFolder]/Bundle" target="[UserCommon]/Adobe/Test/Bundle" recursive="true"/>
        </Assets></Package>"#;
        let context = test_context(staging.path(), home.path());
        install_pimx(xml, &context).unwrap();
        let target = home
            .path()
            .join("Library/Application Support/Adobe/Test/Bundle");
        assert!(
            fs::metadata(target.join("bin.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
                != 0
        );
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            PathBuf::from("bin.sh")
        );
    }

    #[test]
    fn rejects_target_traversal() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let xml = r#"<Package><Assets>
            <Asset source="[StagingFolder]/x" target="[UserPreferences]/../../../evil.txt"/>
        </Assets></Package>"#;
        let context = test_context(staging.path(), home.path());
        let error = install_pimx(xml, &context).unwrap_err();
        assert!(error.to_string().contains("穿越"));
    }

    #[test]
    fn rejects_unknown_command() {
        let staging = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let xml = r#"<Package><Commands><MysteryCommand><Path>[INSTALLDIR]</Path></MysteryCommand></Commands></Package>"#;
        let context = test_context(staging.path(), home.path());
        let error = install_pimx(xml, &context).unwrap_err();
        assert!(error.to_string().contains("不支持的 PIMX 命令"));
    }
}
