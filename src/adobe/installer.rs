use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use super::database;
use super::model::{Application, DownloadedProduct};
use super::{pimx, system};

use crate::macos::process;

mod dmg;
mod payload;

pub fn install_products(
    products: &[DownloadedProduct],
    destination: &Path,
    language: &str,
) -> Result<PathBuf> {
    if products.is_empty() {
        bail!("没有待安装的 Adobe 产品");
    }
    let architecture = current_architecture().context("获取系统架构")?;
    let home = home_path().context("获取用户主目录")?;
    let mut application_xml = None;
    let mut main_path = None;
    for (index, product) in products.iter().enumerate() {
        let application = &product.application;
        eprintln!("安装 {} ({})", application.name, application.sap_code);
        if application.is_dmg {
            dmg::install_dmg_package(product)?;
            continue;
        }
        let mut context = pimx::InstallContext::new(
            &application.install_dir,
            destination,
            &home,
            &product.directory,
            &application.base_version,
            language,
            &architecture,
        )
        .with_context(|| format!("解析安装目录 {}", application.install_dir))?;
        if system::is_system_path(&context.install_dir) {
            process::run(Command::new("sudo").arg("-v"), "缓存管理员授权")?;
        }
        for downloaded in &product.packages {
            let package = &downloaded.package;
            let extension = downloaded
                .path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if extension == "dmg" {
                dmg::install_dmg_package(product)
                    .with_context(|| format!("安装 {}", package.name))?;
                continue;
            }
            if let Some(xml) = payload::install_zip_package(downloaded, &mut context)? {
                application_xml = Some(xml);
            }
        }
        if index == products.len() - 1 {
            main_path = Some(main_app_path(application, &context.install_dir)?);
        }
    }
    let main = products.last().expect("产品列表非空");
    if main.application.is_dmg {
        // DMG/PKG 安装器自带注册逻辑，跳过 ZIP 流程的配置与数据库写入
        let main_path = acrobat_main_path()?;
        crate::macos::apps::clear_extended_attributes(&main_path)?;
        return Ok(main_path);
    }
    let main_path = main_path.expect("循环必然给 main_path 赋值");
    let username = std::env::var("USER").context("获取当前用户名")?;
    save_config(main, &main_path, &username, application_xml.as_deref())?;
    let main_xml = application_xml.as_deref();
    if !system::is_root() {
        // 非特权运行的系统数据库写入需要授权，失败不阻断已完成的安装
        if let Err(error) = database::register(products, language, main_xml) {
            eprintln!("警告: Adobe 数据库注册失败，安装仍已完成: {error:#}");
        }
    } else {
        database::register(products, language, main_xml).context("写入 Adobe 安装数据库")?;
    }
    crate::macos::apps::clear_extended_attributes(&main_path)?;

    Ok(main_path)
}

fn map_architecture(uname: &str) -> &'static str {
    if uname.contains("arm") || uname == "aarch64" {
        "arm64"
    } else {
        "x64"
    }
}

fn current_architecture() -> Result<String> {
    let output = process::output(Command::new("uname").arg("-m"), "uname")?;

    Ok(map_architecture(String::from_utf8_lossy(&output.stdout).trim()).to_owned())
}

fn home_path() -> Result<PathBuf> {
    if system::is_root() {
        Ok(PathBuf::from("/var/root"))
    } else {
        std::env::var("HOME")
            .map(PathBuf::from)
            .context("获取 HOME 环境变量")
    }
}

// app_launch 相对 INSTALLDIR 的首个 .app 组件即主应用
fn main_app_path(application: &Application, install_dir: &Path) -> Result<PathBuf> {
    let relative = application
        .app_launch
        .replace("[INSTALLDIR]/", "")
        .replace("[InstallDir]/", "");
    let Some(first) = Path::new(&relative).components().next() else {
        bail!("app_launch 未提供相对路径: {}", application.app_launch);
    };
    let Component::Normal(name) = first else {
        bail!("app_launch 相对路径无效: {}", application.app_launch);
    };
    let app = install_dir.join(name);
    if !app.extension().is_some_and(|ext| ext == "app") {
        bail!("app_launch 首个组件不是 .app: {}", app.display());
    }
    if !app.exists() {
        bail!("主应用不存在: {}", app.display());
    }
    Ok(app)
}

pub(crate) fn acrobat_candidates() -> [PathBuf; 2] {
    [
        PathBuf::from("/Applications/Adobe Acrobat DC/Adobe Acrobat.app"),
        PathBuf::from("/Applications/Adobe Acrobat Reader DC.app"),
    ]
}

fn acrobat_main_path() -> Result<PathBuf> {
    acrobat_candidates()
        .into_iter()
        .find(|path| path.is_dir())
        .context("installer 已退出，但未找到 Adobe Acrobat 主应用")
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProductConfig {
    application_xml_path: String,
    supported_languages: Vec<String>,
    username: String,
    global_support_paths: Vec<String>,
    user_paths: Vec<String>,
}

fn save_config(
    product: &DownloadedProduct,
    main_path: &Path,
    username: &str,
    application_xml: Option<&Path>,
) -> Result<()> {
    let application = &product.application;
    let Some(xml_path) = application_xml else {
        bail!("缺少 application.xml，无法保存产品配置");
    };
    let config = ProductConfig {
        application_xml_path: xml_path.display().to_string(),
        supported_languages: application.supported_languages.clone(),
        username: username.to_owned(),
        global_support_paths: vec![
            "/Library/Application Support/Adobe".to_owned(),
            "/Library/Preferences".to_owned(),
        ],
        user_paths: vec![format!(
            "{}/Library/Application Support/Adobe",
            home_path()?.display()
        )],
    };
    let target = main_path
        .join("Contents")
        .join("Resources")
        .join("qiuchen_config.json");
    let content = serde_json::to_string_pretty(&config).context("序列化产品配置")?;
    let staging = tempfile::tempdir().context("创建配置暂存目录")?;
    let staged = staging.path().join("qiuchen_config.json");
    fs::write(&staged, content).with_context(|| format!("写入 {}", staged.display()))?;
    system::install_file(&staged, &target).context("保存产品配置")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures;
    use super::*;

    fn application() -> Application {
        Application {
            family_name: Some("Photoshop".to_owned()),
            product_version: "28.0.0.20".to_owned(),
            base_version: "28.0".to_owned(),
            install_dir: "[AdobeProgramFiles]/Adobe Photoshop 2027".to_owned(),
            app_launch: "[INSTALLDIR]/Adobe Photoshop 2027.app/Contents/MacOS/Adobe Photoshop 2027"
                .to_owned(),
            supported_languages: vec!["en_US".to_owned(), "zh_CN".to_owned()],
            cdn: "https://ccmdls.adobe.com".to_owned(),
            ..fixtures::application("PHSP", "Photoshop")
        }
    }

    #[test]
    fn maps_uname_to_adobe_architecture() {
        assert_eq!(map_architecture("arm64"), "arm64");
        assert_eq!(map_architecture("aarch64"), "arm64");
        assert_eq!(map_architecture("x86_64"), "x64");
        assert_eq!(map_architecture("amd64"), "x64");
    }

    #[test]
    fn derives_main_app_from_app_launch() {
        let dir = tempfile::tempdir().unwrap();
        let install_dir = dir.path().join("Adobe Photoshop 2027");
        let app = install_dir.join("Adobe Photoshop 2027.app");
        fs::create_dir_all(&app).unwrap();
        let path = main_app_path(&application(), &install_dir).unwrap();
        assert_eq!(path, app);
    }

    #[test]
    fn rejects_non_app_launch_prefix() {
        let expected = application();
        assert!(main_app_path(&expected, Path::new("/tmp/x")).is_err());
        let mut renamed = expected;
        renamed.app_launch = "[INSTALLDIR]/bin/tool".to_owned();
        assert!(main_app_path(&renamed, Path::new("/tmp/x")).is_err());
    }

    #[test]
    fn saves_product_config() {
        let dir = tempfile::tempdir().unwrap();
        let app_dir = dir.path().join("App.app");
        let resources = app_dir.join("Contents/Resources");
        fs::create_dir_all(&resources).unwrap();
        let main_xml = dir.path().join("application.xml");
        fs::write(&main_xml, "<Configuration/>").unwrap();
        let product = DownloadedProduct {
            application: application(),
            directory: dir.path().to_path_buf(),
            packages: Vec::new(),
        };
        save_config(&product, &app_dir, "tester", Some(&main_xml)).unwrap();
        let config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(resources.join("qiuchen_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["applicationXmlPath"], main_xml.display().to_string());
        assert_eq!(config["supportedLanguages"][0], "en_US");
        assert_eq!(config["supportedLanguages"][1], "zh_CN");
        assert_eq!(config["username"], "tester");
        assert_eq!(
            config["globalSupportPaths"][0],
            "/Library/Application Support/Adobe"
        );
        assert!(config["userPaths"][0]
            .as_str()
            .unwrap()
            .ends_with("/Library/Application Support/Adobe"));
    }
}
