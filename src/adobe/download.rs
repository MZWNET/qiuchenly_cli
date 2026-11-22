use super::application::{ensure_language, parse_apro_manifest, select_packages};
use super::catalog::CatalogClient;
use super::model::{
    architecture, Application, Dependency, DownloadedPackage, DownloadedProduct, Package, Product,
};
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

use crate::packages::aria2::Downloader;
use crate::packages::checksum::verify_sha256;

pub(super) async fn download_product(
    client: &CatalogClient,
    downloader: &Downloader,
    product: &Product,
    root: &Path,
    language: &str,
) -> Result<DownloadedProduct> {
    let directory = root.join(&product.sap_code);
    fs::create_dir_all(&directory)
        .with_context(|| format!("创建目录 {} 失败", directory.display()))?;

    let (application, application_json) = if product.build_guid.is_empty() {
        let Some(manifest_url) = product.manifest_url.as_deref() else {
            bail!("缺少必需参数: buildGuid 或 manifestURL");
        };
        (
            fetch_apro_manifest(client, product, manifest_url).await?,
            None,
        )
    } else {
        let (application, raw) = client.fetch_application(product).await?;
        (application, Some(raw))
    };
    ensure_language(&application, product, language)?;

    let arch = architecture(&product.platform);
    let os_version = system_os_version()?;
    let headers = client.download_headers();
    let mut packages = Vec::new();
    for package in select_packages(&application, language, arch, &os_version)? {
        let url = package_url(&application.cdn, &package.path)?;
        let path = downloader
            .download_with_headers(&url, &directory, Some(&package.full_name), &headers)
            .await
            .with_context(|| format!("下载 {} 失败", package.full_name))?;
        verify_package(&path, package)
            .with_context(|| format!("校验 {} 失败", package.full_name))?;
        packages.push(DownloadedPackage {
            package: package.clone(),
            path,
        });
    }
    if packages.is_empty() {
        bail!("{} 没有匹配当前语言/架构的安装包", product.sap_code);
    }
    if let Some(json) = application_json {
        fs::write(directory.join("application.json"), json)
            .with_context(|| format!("写入 {} application.json 失败", product.sap_code))?;
    }
    write_driver(root, language, &application, &product.dependencies)
        .with_context(|| format!("更新 {} 失败", root.join("driver.xml").display()))?;
    Ok(DownloadedProduct {
        application,
        directory,
        packages,
    })
}

#[cfg(target_os = "macos")]
fn system_os_version() -> Result<String> {
    let output = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .context("获取系统版本失败")?;
    if !output.status.success() {
        bail!("获取系统版本失败: {}", output.status);
    }
    Ok(String::from_utf8(output.stdout)
        .context("系统版本不是有效的 UTF-8")?
        .trim()
        .to_owned())
}

#[cfg(not(target_os = "macos"))]
fn system_os_version() -> Result<String> {
    Ok("0.0".to_owned())
}

fn package_url(cdn: &str, path: &str) -> Result<String> {
    if path.starts_with("http://") || path.starts_with("https://") {
        return Ok(path.to_owned());
    }
    if path.is_empty() {
        bail!("安装包路径为空");
    }
    Ok(format!(
        "{}/{}",
        cdn.trim_end_matches('/'),
        path.trim_start_matches('/')
    ))
}

fn verify_package(path: &Path, package: &Package) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("读取 {} 元数据失败", path.display()))?;
    if let Some(expected) = package.download_size.filter(|size| *size > 0) {
        if metadata.len() != expected {
            bail!(
                "{} 大小校验失败: 期望 {expected} 字节，实际 {} 字节",
                package.full_name,
                metadata.len()
            );
        }
    }
    if let Some(expected) = package
        .sha256
        .as_deref()
        .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
    {
        verify_sha256(path, expected, &package.full_name)?;
    }
    Ok(())
}

async fn fetch_apro_manifest(
    client: &CatalogClient,
    product: &Product,
    manifest_url: &str,
) -> Result<Application> {
    let url = package_url(&product.cdn, manifest_url)?;
    let xml = client
        .get_text(&url)
        .await
        .with_context(|| format!("获取 {} manifest 失败", product.sap_code))?;
    parse_apro_manifest(&xml, product)
}

// 原版 driver.xml 结构: DriverInfo → ProductInfo(Name/SAPCode/CodexVersion/Platform/EsdDirectory/Dependencies)
// 与 RequestInfo(InstallDir/InstallLanguage)，主产品在最后一次调用覆盖整份文件
fn write_driver(
    root: &Path,
    language: &str,
    application: &Application,
    dependencies: &[Dependency],
) -> Result<()> {
    let escape = escape_xml;
    let mut entries = String::new();
    for dependency in dependencies {
        entries.push_str(&format!(
            "      <Dependency>\n\
             \x20       <SAPCode>{}</SAPCode>\n\
             \x20       <BaseVersion>{}</BaseVersion>\n\
             \x20       <EsdDirectory>{}</EsdDirectory>\n\
             \x20     </Dependency>\n",
            escape(&dependency.sap_code),
            escape(&dependency.base_version),
            escape(&dependency.sap_code),
        ));
    }
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <DriverInfo>\n\
         \x20 <ProductInfo>\n\
         \x20   <Name>{name}</Name>\n\
         \x20   <SAPCode>{sap}</SAPCode>\n\
         \x20   <CodexVersion>{version}</CodexVersion>\n\
         \x20   <Platform>{platform}</Platform>\n\
         \x20   <EsdDirectory>{esd}</EsdDirectory>\n\
         \x20   <Dependencies>\n{entries}    </Dependencies>\n\
         \x20 </ProductInfo>\n\
         \x20 <RequestInfo>\n\
         \x20   <InstallDir>{install_dir}</InstallDir>\n\
         \x20   <InstallLanguage>{language}</InstallLanguage>\n\
         \x20 </RequestInfo>\n\
         </DriverInfo>\n",
        name = escape(&application.name),
        sap = escape(&application.sap_code),
        version = escape(&application.base_version),
        platform = escape(&application.platform),
        esd = escape(&application.sap_code),
        install_dir = escape(&application.install_dir),
        language = escape(language),
    );
    let path = root.join("driver.xml");
    fs::write(&path, xml).with_context(|| format!("写入 {} 失败", path.display()))
}

fn escape_xml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures::{application, package};
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn package_url_joins_cdn_or_uses_absolute() {
        assert_eq!(
            package_url("https://cdn.example.com", "/AdobeProducts/a.zip").unwrap(),
            "https://cdn.example.com/AdobeProducts/a.zip"
        );
        assert_eq!(
            package_url(
                "https://cdn.example.com/",
                "https://other.example.com/a.zip"
            )
            .unwrap(),
            "https://other.example.com/a.zip"
        );
        assert!(package_url("https://cdn.example.com", "").is_err());
    }

    #[test]
    fn verifies_download_size_and_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.zip");
        fs::write(&file, b"hello").unwrap();
        let mut pkg = package("hello", 1, None);
        pkg.download_size = Some(5);
        assert!(verify_package(&file, &pkg).is_ok());
        pkg.download_size = Some(4);
        assert!(verify_package(&file, &pkg).is_err());
        pkg.download_size = None;
        pkg.sha256 =
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824".to_owned());
        assert!(verify_package(&file, &pkg).is_ok());
        pkg.sha256 = Some("a".repeat(64));
        assert!(verify_package(&file, &pkg).is_err());
        // 旧目录使用 32 位十六进制 MD5，无法用 SHA256 校验，跳过
        pkg.sha256 = Some("a1a3139a60dcf12638d4b5f0b38aa7b6".to_owned());
        assert!(verify_package(&file, &pkg).is_ok());
    }

    #[test]
    fn verifies_sha256_of_large_file_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("big.zip");
        let contents = vec![7u8; 1024 * 1024 + 1];
        fs::write(&file, &contents).unwrap();
        let digest: String = Sha256::digest(&contents)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let mut pkg = package("big", 1, None);
        pkg.sha256 = Some(digest);
        assert!(verify_package(&file, &pkg).is_ok());
    }

    #[test]
    fn driver_xml_matches_original_layout() {
        let root = tempfile::tempdir().unwrap();
        let app = application("PHSP", "Photoshop \"26\"");
        write_driver(root.path(), "zh_CN", &app, &[]).unwrap();
        let xml = fs::read_to_string(root.path().join("driver.xml")).unwrap();
        assert!(xml.contains("<SAPCode>PHSP</SAPCode>"));
        assert!(xml.contains("<CodexVersion>1.0</CodexVersion>"));
        assert!(xml.contains("<InstallLanguage>zh_CN</InstallLanguage>"));
        assert!(xml.contains("Photoshop &quot;26&quot;"));
        assert!(xml.contains("<Dependencies>\n    </Dependencies>"));
    }

    #[test]
    fn driver_xml_lists_dependencies() {
        let root = tempfile::tempdir().unwrap();
        let app = application("PHSP", "Photoshop");
        let dependencies = vec![Dependency {
            sap_code: "CORE & Co".into(),
            base_version: "1.0".into(),
        }];
        write_driver(root.path(), "en_US", &app, &dependencies).unwrap();
        let xml = fs::read_to_string(root.path().join("driver.xml")).unwrap();
        assert!(xml.contains("<SAPCode>CORE &amp; Co</SAPCode>"));
        assert!(xml.contains("<BaseVersion>1.0</BaseVersion>"));
        assert!(xml.contains("<EsdDirectory>CORE &amp; Co</EsdDirectory>"));
    }
}
