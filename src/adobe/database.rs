use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::fs;
use std::path::Path;

use super::model::{Application, DownloadedProduct};
use super::system;

const ADOBE_SUPPORT: &str = "/Library/Application Support/Adobe";

pub(crate) fn register(
    products: &[DownloadedProduct],
    language: &str,
    application_xml: Option<&Path>,
) -> Result<()> {
    let support = Path::new(ADOBE_SUPPORT);
    update_database(&support.join("Adobe PCD").join("hdpim.db"), |connection| {
        populate_hdpim(connection, products, language)
    })
    .context("写入 hdpim.db")?;
    update_database(&support.join("Adobe PCD").join("pcd.db"), |connection| {
        populate_pcd(connection, products, application_xml)
    })
    .context("写入 pcd.db")?;
    update_database(&support.join("Adobe PCD").join("pdb.db"), |connection| {
        populate_pdb(connection, products)
    })
    .context("写入 pdb.db")?;
    if let (Some(xml), Some(main)) = (application_xml, products.last()) {
        let target = support
            .join("PCF")
            .join(format!("{}.xml", main.application.sap_code));
        system::install_file(xml, &target).context("复制 application.xml 到 PCF")?;
    }
    Ok(())
}

// 先在 tempdir 中更新（已存在时复制出来改），再落到系统目录
fn update_database(system: &Path, populate: impl FnOnce(&Connection) -> Result<()>) -> Result<()> {
    let staging = tempfile::tempdir().context("创建数据库暂存目录")?;
    let staged = staging
        .path()
        .join(system.file_name().context("数据库路径必须有文件名")?);
    if system.exists() {
        fs::copy(system, &staged)
            .with_context(|| format!("复制现有数据库 {}", system.display()))?;
    }
    let connection =
        Connection::open(&staged).with_context(|| format!("打开数据库 {}", staged.display()))?;
    populate(&connection)?;
    drop(connection);
    system::install_file(&staged, system)
}

fn populate_hdpim(
    connection: &Connection,
    products: &[DownloadedProduct],
    language: &str,
) -> Result<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS pim_meta (Key TEXT NOT NULL, Value TEXT NOT NULL, PRIMARY KEY (Key));
             CREATE TABLE IF NOT EXISTS product_installation_info (SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, Status INTEGER NOT NULL, PRIMARY KEY (SAPCode, ProductVersion, ProcessorFamily));
             CREATE TABLE IF NOT EXISTS product_installation_meta_info (SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, Key TEXT NOT NULL, Value TEXT, PRIMARY KEY (SAPCode, ProductVersion, ProcessorFamily, Key));
             CREATE TABLE IF NOT EXISTS package_installation_info (SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, PackageName TEXT NOT NULL, PackageVersion TEXT NOT NULL, PRIMARY KEY (SAPCode, ProductVersion, ProcessorFamily, PackageName, PackageVersion));
             CREATE TABLE IF NOT EXISTS package_installation_meta_info (SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, PackageName TEXT NOT NULL, PackageVersion TEXT NOT NULL, Key TEXT NOT NULL, Value TEXT, PRIMARY KEY (SAPCode, ProductVersion, ProcessorFamily, PackageName, PackageVersion, Key));
             CREATE TABLE IF NOT EXISTS product_reference_info (SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, ReferencingSAPCode TEXT NOT NULL, ReferencingProductVersion TEXT NOT NULL, ReferencingProcessorFamily TEXT NOT NULL, Type TEXT);
             CREATE TABLE IF NOT EXISTS Package_reference_info (PackageName TEXT NOT NULL, PackageVersion TEXT NOT NULL, SAPCode TEXT NOT NULL, ProductVersion TEXT NOT NULL, ProcessorFamily TEXT NOT NULL, Type TEXT, PRIMARY KEY (PackageName, PackageVersion, SAPCode, ProductVersion, ProcessorFamily));",
        )
        .context("创建 hdpim 表")?;
    connection
        .execute(
            "INSERT OR REPLACE INTO pim_meta (Key, Value) VALUES ('schema_version', '2')",
            [],
        )
        .context("写入 pim_meta")?;
    connection
        .execute(
            "INSERT OR REPLACE INTO pim_meta (Key, Value) VALUES ('schema_compatibility_version', '1')",
            [],
        )
        .context("写入 pim_meta")?;
    for product in products {
        register_hdpim_product(connection, product, language)?;
    }
    if let Some((main, dependencies)) = products.split_last() {
        for dependency in dependencies {
            connection
                .execute(
                    "INSERT OR REPLACE INTO product_reference_info (SAPCode, ProductVersion, ProcessorFamily, ReferencingSAPCode, ReferencingProductVersion, ReferencingProcessorFamily, Type) VALUES (?, ?, ?, ?, ?, ?, '')",
                    params![
                        dependency.application.sap_code,
                        dependency.application.product_version,
                        processor_family(&dependency.application.platform),
                        main.application.sap_code,
                        main.application.product_version,
                        processor_family(&main.application.platform)
                    ],
                )
                .context("写入 product_reference_info")?;
        }
    }
    Ok(())
}

fn register_hdpim_product(
    connection: &Connection,
    product: &DownloadedProduct,
    language: &str,
) -> Result<()> {
    let application = &product.application;
    let family = processor_family(&application.platform);
    connection
        .execute(
            "INSERT OR REPLACE INTO product_installation_info (SAPCode, ProductVersion, ProcessorFamily, Status) VALUES (?, ?, ?, 1)",
            params![application.sap_code, application.product_version, family],
        )
        .context("写入 product_installation_info")?;
    for (key, value) in product_meta(application, language) {
        connection
            .execute(
                "INSERT OR REPLACE INTO product_installation_meta_info (SAPCode, ProductVersion, ProcessorFamily, Key, Value) VALUES (?, ?, ?, ?, ?)",
                params![application.sap_code, application.product_version, family, key, value],
            )
            .with_context(|| format!("写入产品元信息 {key}"))?;
    }
    for downloaded in &product.packages {
        let package = &downloaded.package;
        connection
            .execute(
                "INSERT OR REPLACE INTO package_installation_info (SAPCode, ProductVersion, ProcessorFamily, PackageName, PackageVersion) VALUES (?, ?, ?, ?, ?)",
                params![application.sap_code, application.product_version, family, package.name, package.version],
            )
            .with_context(|| format!("写入包信息 {}", package.name))?;
        connection
            .execute(
                "INSERT OR REPLACE INTO package_installation_meta_info (SAPCode, ProductVersion, ProcessorFamily, PackageName, PackageVersion, Key, Value) VALUES (?, ?, ?, ?, ?, 'ProcessorFamily', ?)",
                params![application.sap_code, application.product_version, family, package.name, package.version, family],
            )
            .with_context(|| format!("写入包元信息 {}", package.name))?;
        connection
            .execute(
                "INSERT OR REPLACE INTO Package_reference_info (PackageName, PackageVersion, SAPCode, ProductVersion, ProcessorFamily, Type) VALUES (?, ?, ?, ?, ?, '')",
                params![package.name, package.version, application.sap_code, application.product_version, family],
            )
            .context("写入 Package_reference_info")?;
    }
    Ok(())
}

fn product_meta(application: &Application, language: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Name", application.name.clone()),
        ("Platform", application.platform.clone()),
        ("InstallLang", language.to_owned()),
        ("BaseVersion", application.base_version.clone()),
        ("BuildVersion", application.product_version.clone()),
        ("isUWPProduct", "false".to_owned()),
        ("isVisibleProduct", "true".to_owned()),
        ("isNonCCProduct", "false".to_owned()),
        ("isSelfReference", "false".to_owned()),
        ("autoInstall", "false".to_owned()),
        ("autoPatchUpdate", "false".to_owned()),
    ]
}

fn populate_pcd(
    connection: &Connection,
    products: &[DownloadedProduct],
    application_xml: Option<&Path>,
) -> Result<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS pcd_meta (key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY (key));
             CREATE TABLE IF NOT EXISTS domain_data (domain TEXT NOT NULL, subDomain TEXT NOT NULL, key TEXT NOT NULL, value TEXT, PRIMARY KEY (domain, subDomain, key));",
        )
        .context("创建 pcd 表")?;
    connection
        .execute(
            "INSERT OR REPLACE INTO pcd_meta (key, value) VALUES ('schema_version', '2')",
            [],
        )
        .context("写入 pcd_meta")?;
    connection
        .execute(
            "INSERT OR REPLACE INTO pcd_meta (key, value) VALUES ('schema_compatibility_version', '1')",
            [],
        )
        .context("写入 pcd_meta")?;
    if let (Some(xml), Some(main)) = (application_xml, products.last()) {
        let sap_code = &main.application.sap_code;
        for (key, value) in parse_application_xml(xml)? {
            connection
                .execute(
                    "INSERT OR REPLACE INTO domain_data (domain, subDomain, key, value) VALUES (1, ?, ?, ?)",
                    params![sap_code, key, value],
                )
                .with_context(|| format!("写入 PCD 域数据 {key}"))?;
        }
    }
    Ok(())
}

fn parse_application_xml(path: &Path) -> Result<Vec<(String, String)>> {
    let xml = fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    let document = roxmltree::Document::parse(&xml).context("解析 application.xml")?;
    let root = document.root_element();
    if root.tag_name().name() != "Configuration" {
        bail!("application.xml 未找到 Configuration 根节点");
    }
    let payload = root
        .children()
        .find(|node| node.has_tag_name("Payload"))
        .context("application.xml 未找到 Payload 节点")?;
    let mut entries = Vec::new();
    for data in payload.children().filter(|node| node.has_tag_name("Data")) {
        if let Some(key) = data.attribute("key") {
            entries.push((key.to_owned(), data.text().unwrap_or_default().to_owned()));
        }
    }
    Ok(entries)
}

fn populate_pdb(connection: &Connection, products: &[DownloadedProduct]) -> Result<()> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS payloads (PayloadID TEXT NOT NULL, productFamily TEXT NOT NULL, productName TEXT NOT NULL, version TEXT NOT NULL, installState INTEGER NOT NULL, installTime INTEGER NOT NULL, PRIMARY KEY (PayloadID));
             CREATE TABLE IF NOT EXISTS install_source (product_id TEXT NOT NULL, source_path TEXT NOT NULL, install_time INTEGER NOT NULL, PRIMARY KEY (product_id, source_path));",
        )
        .context("创建 pdb 表")?;
    let install_time = Utc::now().timestamp();
    for product in products {
        let application = &product.application;
        let family = application
            .family_name
            .as_deref()
            .unwrap_or(&application.name);
        for downloaded in &product.packages {
            let package = &downloaded.package;
            let source_path = downloaded.path.display().to_string();
            connection
                .execute(
                    "INSERT OR REPLACE INTO payloads (PayloadID, productFamily, productName, version, installState, installTime) VALUES (?, ?, ?, ?, 2, ?)",
                    params![package.alias_name.as_deref().unwrap_or(&package.name), family, application.name, package.version, install_time],
                )
                .with_context(|| format!("写入 payloads {}", package.name))?;
            connection
                .execute(
                    "DELETE FROM install_source WHERE product_id = ? AND source_path = ?",
                    params![application.sap_code, source_path],
                )
                .context("清理 install_source")?;
            connection
                .execute(
                    "INSERT INTO install_source (product_id, source_path, install_time) VALUES (?, ?, ?)",
                    params![application.sap_code, source_path, install_time],
                )
                .with_context(|| format!("写入 install_source {}", package.name))?;
        }
    }
    Ok(())
}

fn processor_family(platform: &str) -> &'static str {
    if platform.contains("64") || platform.contains("universal") {
        "64"
    } else {
        "32"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adobe::model::{fixtures, DownloadedPackage, Package};
    use std::path::PathBuf;

    fn application(sap_code: &str, name: &str, product_version: &str) -> Application {
        Application {
            product_version: product_version.to_owned(),
            base_version: product_version
                .split('.')
                .next()
                .unwrap_or_default()
                .to_owned()
                + ".0",
            install_dir: format!("[AdobeProgramFiles]/{name}"),
            app_launch: format!("[INSTALLDIR]/{name}.app/Contents/MacOS/{name}"),
            supported_languages: vec!["en_US".to_owned()],
            cdn: "https://ccmdls.adobe.com".to_owned(),
            ..fixtures::application(sap_code, name)
        }
    }

    fn downloaded_product(
        sap_code: &str,
        name: &str,
        version: &str,
        package: &str,
    ) -> DownloadedProduct {
        let package = Package {
            version: version.to_owned(),
            full_name: package.to_owned(),
            path: format!("/AdobeProducts/{sap_code}/{package}.zip"),
            ..fixtures::package(package, 7000, None)
        };
        let path = PathBuf::from(format!("/tmp/{}.zip", package.name));
        DownloadedProduct {
            application: application(sap_code, name, version),
            directory: PathBuf::from("/tmp"),
            packages: vec![DownloadedPackage { package, path }],
        }
    }

    fn fixture_products() -> Vec<DownloadedProduct> {
        vec![
            downloaded_product(
                "CORE",
                "STI_Color_HD",
                "1.0.12",
                "AdobeColorEU_ExtraSettings_1_0-mul",
            ),
            downloaded_product("PHSP", "Photoshop", "28.0.0.20", "AdobePhotoshop28-Core"),
        ]
    }

    #[test]
    fn hdpim_registration_round_trips() {
        let connection = Connection::open_in_memory().unwrap();
        let products = fixture_products();
        populate_hdpim(&connection, &products, "en_US").unwrap();
        let status: i64 = connection
            .query_row(
                "SELECT Status FROM product_installation_info WHERE SAPCode = 'PHSP'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, 1);
        let schema: String = connection
            .query_row(
                "SELECT Value FROM pim_meta WHERE Key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema, "2");
        let packages: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM package_installation_info",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(packages, 2);
        let referencing: String = connection
            .query_row(
                "SELECT ReferencingSAPCode FROM product_reference_info WHERE SAPCode = 'CORE'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(referencing, "PHSP");
        let name: String = connection
            .query_row(
                "SELECT Value FROM product_installation_meta_info WHERE SAPCode = 'PHSP' AND Key = 'Name'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Photoshop");
    }

    #[test]
    fn pcd_domain_data_reads_application_xml() {
        let dir = tempfile::tempdir().unwrap();
        let xml = dir.path().join("application.xml");
        fs::write(
            &xml,
            r#"<?xml version="1.0" encoding="UTF-8"?><Configuration><Payload><Data key="adobeCode">V7{}Photoshop-28-Mac-GM</Data><Data key="installedLanguages">en_US</Data></Payload></Configuration>"#,
        )
        .unwrap();
        let connection = Connection::open_in_memory().unwrap();
        let products = fixture_products();
        populate_pcd(&connection, &products, Some(&xml)).unwrap();
        let code: String = connection
            .query_row(
                "SELECT value FROM domain_data WHERE key = 'adobeCode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code, "V7{}Photoshop-28-Mac-GM");
        let sub_domain: String = connection
            .query_row(
                "SELECT subDomain FROM domain_data WHERE key = 'installedLanguages'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sub_domain, "PHSP");
        let compatibility: String = connection
            .query_row(
                "SELECT value FROM pcd_meta WHERE key = 'schema_compatibility_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(compatibility, "1");
    }

    #[test]
    fn pdb_payloads_and_install_source_round_trip() {
        let connection = Connection::open_in_memory().unwrap();
        let mut products = fixture_products();
        products[1].application.family_name = Some("Creative Cloud".to_owned());
        products[1].packages[0].package.alias_name = Some("PHSP-alias".to_owned());
        populate_pdb(&connection, &products).unwrap();
        let (state, family, name): (i64, String, String) = connection
            .query_row(
                "SELECT installState, productFamily, productName FROM payloads WHERE PayloadID = 'PHSP-alias'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, 2);
        assert_eq!(family, "Creative Cloud");
        assert_eq!(name, "Photoshop");
        let fallback_family: String = connection
            .query_row(
                "SELECT productFamily FROM payloads WHERE PayloadID = 'AdobeColorEU_ExtraSettings_1_0-mul'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fallback_family, "STI_Color_HD");
        let sources: i64 = connection
            .query_row("SELECT COUNT(*) FROM install_source", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sources, 2);
    }

    #[test]
    fn derives_processor_family_from_platform() {
        assert_eq!(processor_family("macuniversal"), "64");
        assert_eq!(processor_family("osx10-64"), "64");
        assert_eq!(processor_family("osx10"), "32");
    }
}
