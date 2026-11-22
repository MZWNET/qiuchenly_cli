use super::condition;
use super::model::{Application, Package, Product};
use super::response::OneOrMany;
use anyhow::{bail, Context, Result};
use roxmltree::{Document, Node};
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawApplication {
    #[serde(default)]
    name: String,
    #[serde(rename = "SAPCode", default)]
    sap_code: String,
    #[serde(default)]
    product_version: String,
    #[serde(default)]
    base_version: String,
    #[serde(default)]
    platform: String,
    #[serde(default)]
    family_name: Option<String>,
    #[serde(default)]
    cdn: Option<RawApplicationCdn>,
    #[serde(default)]
    install_dir: Option<RawInstallDir>,
    #[serde(default)]
    app_launch: String,
    #[serde(default)]
    supported_languages: RawSupportedLanguages,
    #[serde(default)]
    packages: RawPackages,
    #[serde(default)]
    modules: Option<RawModules>,
}

#[derive(Deserialize)]
struct RawApplicationCdn {
    #[serde(default, rename = "Secure")]
    secure: String,
}

#[derive(Deserialize)]
struct RawInstallDir {
    #[serde(default)]
    value: String,
}

#[derive(Default, Deserialize)]
struct RawSupportedLanguages {
    #[serde(default, rename = "Language")]
    language: OneOrMany<RawLanguage>,
}

#[derive(Default, Deserialize)]
struct RawLanguage {
    #[serde(default)]
    locale: String,
}

#[derive(Default, Deserialize)]
struct RawPackages {
    #[serde(default, rename = "Package")]
    package: OneOrMany<RawPackage>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPackage {
    #[serde(default)]
    package_name: String,
    #[serde(default)]
    package_version: String,
    #[serde(default)]
    path: String,
    #[serde(default, rename = "fullPackageName")]
    full_package_name: String,
    #[serde(default)]
    condition: Option<String>,
    #[serde(default)]
    download_size: Option<u64>,
    #[serde(default, rename = "packageHashKey")]
    package_hash_key: Option<String>,
    #[serde(default, rename = "InstallSequenceNumber")]
    install_sequence: u64,
    #[serde(default, rename = "AliasPackageName")]
    alias_name: Option<String>,
}

#[derive(Default, Deserialize)]
struct RawModules {
    #[serde(default, rename = "Module")]
    module: OneOrMany<RawModule>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawModule {
    #[serde(default)]
    deployment_type: Option<String>,
    #[serde(default)]
    reference_packages: RawReferencePackages,
}

#[derive(Default, Deserialize)]
struct RawReferencePackages {
    #[serde(default, rename = "ReferencePackage")]
    reference_package: OneOrMany<RawReferencePackage>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawReferencePackage {
    Name(String),
    Detailed {
        #[serde(rename = "PackageName")]
        package_name: String,
    },
}

fn deferred_package_names(modules: &Option<RawModules>) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(modules) = modules else {
        return names;
    };
    for module in modules.module.iter() {
        if !matches!(
            module.deployment_type.as_deref(),
            Some("Deferred") | Some("OnDemand")
        ) {
            continue;
        }
        for reference in module.reference_packages.reference_package.iter() {
            match reference {
                RawReferencePackage::Name(name) => {
                    names.insert(name.clone());
                }
                RawReferencePackage::Detailed { package_name } => {
                    names.insert(package_name.clone());
                }
            }
        }
    }
    names
}

pub(super) fn parse_application(raw: &str, product: &Product) -> Result<Application> {
    let application: RawApplication =
        serde_json::from_str(raw).context("Adobe application.json 解析失败")?;
    let cdn = application
        .cdn
        .as_ref()
        .map(|cdn| cdn.secure.clone())
        .filter(|cdn| !cdn.is_empty())
        .unwrap_or_else(|| product.cdn.clone());
    let packages: Vec<Package> = application
        .packages
        .package
        .iter()
        .map(|package| Package {
            name: package.package_name.clone(),
            version: if package.package_version.is_empty() {
                product.base_version.clone()
            } else {
                package.package_version.clone()
            },
            full_name: if package.full_package_name.is_empty() {
                package
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            } else {
                package.full_package_name.clone()
            },
            path: package.path.clone(),
            condition: package.condition.clone(),
            download_size: package.download_size,
            sha256: package.package_hash_key.clone(),
            install_sequence: package.install_sequence,
            alias_name: package.alias_name.clone(),
        })
        .collect();
    let supported_languages: Vec<String> = application
        .supported_languages
        .language
        .iter()
        .map(|language| language.locale.clone())
        .filter(|locale| !locale.is_empty())
        .collect();
    Ok(Application {
        sap_code: application.sap_code,
        name: application.name,
        family_name: application.family_name,
        product_version: application.product_version,
        base_version: if application.base_version.is_empty() {
            product.base_version.clone()
        } else {
            application.base_version
        },
        platform: if application.platform.is_empty() {
            product.platform.clone()
        } else {
            application.platform
        },
        install_dir: application
            .install_dir
            .map(|dir| dir.value)
            .unwrap_or_default(),
        app_launch: application.app_launch,
        supported_languages,
        cdn,
        packages,
        deferred_packages: deferred_package_names(&application.modules),
        is_dmg: false,
    })
}

// 依赖组件多为多语言/ALL，只有语言受限的产品才需要校验
pub(super) fn ensure_language(
    application: &Application,
    product: &Product,
    language: &str,
) -> Result<()> {
    if application.supported_languages.is_empty()
        || application.supported_languages.iter().any(|l| l == "ALL")
    {
        return Ok(());
    }
    if application
        .supported_languages
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(language))
    {
        return Ok(());
    }
    bail!(
        "{} 不支持语言 {language}，支持: {}",
        product.sap_code,
        application.supported_languages.join(", ")
    )
}

pub(super) fn select_packages<'a>(
    application: &'a Application,
    language: &str,
    architecture: &str,
    os_version: &str,
) -> Result<Vec<&'a Package>> {
    let mut selected = Vec::new();
    for package in &application.packages {
        if application.deferred_packages.contains(&package.name) {
            continue;
        }
        if let Some(expr) = &package.condition {
            if !condition::matches(expr, language, architecture, os_version)
                .with_context(|| format!("评估包 {} 条件失败", package.name))?
            {
                continue;
            }
        }
        selected.push(package);
    }
    selected.sort_by_key(|package| package.install_sequence);
    Ok(selected)
}

pub(super) fn parse_apro_manifest(xml: &str, product: &Product) -> Result<Application> {
    let document =
        Document::parse(xml).with_context(|| format!("解析 {} manifest 失败", product.sap_code))?;
    let mut packages = Vec::new();
    for asset in document
        .descendants()
        .filter(|node| node.has_tag_name("asset"))
    {
        if child_text(asset, "asset_type") != Some("DMG") {
            continue;
        }
        let Some(path) = child_text(asset, "asset_path") else {
            continue;
        };
        let full_name = path.rsplit('/').next().unwrap_or_default().to_owned();
        if full_name.is_empty() {
            continue;
        }
        packages.push(Package {
            name: full_name.clone(),
            version: product.version.clone(),
            full_name,
            path: path.to_owned(),
            condition: None,
            download_size: child_text(asset, "asset_size").and_then(|size| size.parse().ok()),
            sha256: None,
            install_sequence: child_text(asset, "sequence_no")
                .and_then(|seq| seq.parse().ok())
                .unwrap_or(packages.len() as u64 + 1),
            alias_name: None,
        });
    }
    if packages.is_empty() {
        bail!("{} manifest 中没有 DMG 资源", product.sap_code);
    }
    Ok(Application {
        sap_code: product.sap_code.clone(),
        name: product.display_name.clone(),
        family_name: None,
        product_version: product.version.clone(),
        base_version: product.base_version.clone(),
        platform: product.platform.clone(),
        install_dir: "[AdobeProgramFiles]".to_owned(),
        app_launch: String::new(),
        supported_languages: Vec::new(),
        cdn: product.cdn.clone(),
        packages,
        deferred_packages: BTreeSet::new(),
        is_dmg: true,
    })
}

fn child_text<'a>(node: Node<'a, '_>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|child| child.has_tag_name(tag))?
        .text()
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures::{application, package};
    use super::*;

    fn product(sap_code: &str) -> Product {
        Product {
            sap_code: sap_code.to_owned(),
            display_name: sap_code.to_owned(),
            version: "1.0".to_owned(),
            base_version: "1.0".to_owned(),
            platform: "macuniversal".to_owned(),
            build_guid: "guid".to_owned(),
            manifest_url: None,
            cdn: "https://cdn.example.com".to_owned(),
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn selects_packages_by_deferred_condition_and_order() {
        let mut deferred = BTreeSet::new();
        deferred.insert("P-Deferred".to_owned());
        let mut app = application("PHSP", "Photoshop");
        app.deferred_packages = deferred;
        app.packages = vec![
            package("P-Late", 3, None),
            package("P-Deferred", 2, Some("[invalid]==value")),
            package("P-Deferred", 2, Some("[invalid]==value")),
            package("P-Arm", 2, Some("[OSArchitecture]==arm64")),
            package("P-Version", 2, Some("[OSVersion]>=14.0")),
            package(
                "P-Lang",
                1,
                Some("[installLanguage]==zh_CN||[installLanguage]==en_US"),
            ),
        ];
        let selected = select_packages(&app, "en_US", "arm64", "14.0").unwrap();
        let names: Vec<&str> = selected.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["P-Lang", "P-Arm", "P-Version", "P-Late"]);
        let selected = select_packages(&app, "de_DE", "x64", "13.0").unwrap();
        assert!(selected.iter().all(|p| p.name == "P-Late"));
    }

    #[test]
    fn parses_apro_manifest_assets() {
        let xml = r#"<manifest><version>1.0</version><asset_list>
          <asset><asset_size>100</asset_size>
            <asset_path>https://cdn.example.com/Acrobat.dmg</asset_path>
            <asset_type>DMG</asset_type><sequence_no>1</sequence_no></asset>
          <asset><asset_size>200</asset_size>
            <asset_path>https://cdn.example.com/other.pkg</asset_path>
            <asset_type>PKG</asset_type></asset>
        </asset_list></manifest>"#;
        let product = Product {
            sap_code: "APRO".into(),
            display_name: "Acrobat".into(),
            version: "26.1".into(),
            base_version: "26.1".into(),
            platform: "macuniversal".into(),
            build_guid: String::new(),
            manifest_url: Some("/AdobeProducts/APRO/26_1/manifest.xml".into()),
            cdn: "https://cdn.example.com".into(),
            dependencies: Vec::new(),
        };
        let app = parse_apro_manifest(xml, &product).unwrap();
        assert!(app.is_dmg);
        assert_eq!(app.install_dir, "[AdobeProgramFiles]");
        assert!(app.supported_languages.is_empty());
        assert_eq!(app.packages.len(), 1);
        let package = &app.packages[0];
        assert_eq!(package.full_name, "Acrobat.dmg");
        assert_eq!(package.path, "https://cdn.example.com/Acrobat.dmg");
        assert_eq!(package.download_size, Some(100));
    }

    #[test]
    fn empty_manifest_is_an_error() {
        let product = product("APRO");
        assert!(
            parse_apro_manifest("<manifest><asset_list></asset_list></manifest>", &product)
                .is_err()
        );
    }

    #[test]
    fn language_validation_accepts_supported_or_unrestricted() {
        let mut app = application("PHSP", "Photoshop");
        assert!(ensure_language(&app, &product("PHSP"), "zh_CN").is_ok());
        app.supported_languages = vec!["en_US".into(), "de_DE".into()];
        assert!(ensure_language(&app, &product("PHSP"), "de_de").is_ok());
        let error = ensure_language(&app, &product("PHSP"), "zh_CN")
            .unwrap_err()
            .to_string();
        assert!(error.contains("zh_CN"), "{error}");
        app.supported_languages = vec!["ALL".into()];
        assert!(ensure_language(&app, &product("PHSP"), "zh_CN").is_ok());
    }

    #[test]
    fn parses_application_json_shapes() {
        let product = Product {
            sap_code: "PHSP".into(),
            display_name: "Photoshop".into(),
            version: "28.0".into(),
            base_version: "28.0".into(),
            platform: "macuniversal".into(),
            build_guid: "phsp-28-guid".into(),
            manifest_url: None,
            cdn: "https://fallback.example.com".into(),
            dependencies: Vec::new(),
        };
        let raw = r#"{
          "Name": "Photoshop", "SAPCode": "PHSP", "ProductVersion": "28.0.0.20",
          "BaseVersion": "28.0", "Platform": "macuniversal", "FamilyName": "Photoshop",
          "Cdn": {"Secure": "https://override.example.com"},
          "InstallDir": {"value": "[AdobeProgramFiles]/Adobe Photoshop 2027"},
          "AppLaunch": "[INSTALLDIR]/Adobe Photoshop 2027.app/Contents/MacOS/Adobe Photoshop 2027",
          "SupportedLanguages": {"Language": [{"locale": "en_US"}, {"locale": "de_DE"}]},
          "Packages": {"Package": [
            {"PackageName": "Z-Last", "PackageVersion": "1.0", "Path": "/p/z.zip",
             "DownloadSize": 5, "packageHashKey": "abc", "InstallSequenceNumber": 2,
             "fullPackageName": "z.zip", "Format": "zip"},
            {"PackageName": "A-First", "Path": "/p/a.zip", "InstallSequenceNumber": 1},
            {"PackageName": "P-Deferred", "Condition": "[invalid]==value"},
            {"PackageName": "P-Deferred", "Condition": "[invalid]==value"},
            {"PackageName": "P-OnDemand", "Condition": "[invalid]==value"}
          ]},
          "Modules": {"Module": [
            {"Id": "m1", "DeploymentType": "Deferred",
             "ReferencePackages": {"ReferencePackage": ["P-Deferred"]}},
            {"Id": "m2", "DeploymentType": "OnDemand",
             "ReferencePackages": {"ReferencePackage": [{"PackageName": "P-OnDemand"}]}},
            {"Id": "m3", "DeploymentType": "Normal",
             "ReferencePackages": {"ReferencePackage": ["P-Normal"]}}
          ]}
        }"#;
        let application = parse_application(raw, &product).unwrap();
        assert_eq!(application.cdn, "https://override.example.com");
        assert_eq!(
            application.install_dir,
            "[AdobeProgramFiles]/Adobe Photoshop 2027"
        );
        assert_eq!(application.supported_languages, ["en_US", "de_DE"]);
        let a_first = application
            .packages
            .iter()
            .find(|package| package.name == "A-First")
            .unwrap();
        assert_eq!(a_first.full_name, "a.zip");
        assert_eq!(a_first.version, "28.0");
        let z_last = application
            .packages
            .iter()
            .find(|package| package.name == "Z-Last")
            .unwrap();
        assert_eq!(z_last.install_sequence, 2);
        let mut deferred = BTreeSet::new();
        deferred.insert("P-Deferred".to_owned());
        deferred.insert("P-OnDemand".to_owned());
        assert_eq!(application.deferred_packages, deferred);
        let selected = select_packages(&application, "en_US", "arm64", "14.0").unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|package| package.name.as_str())
                .collect::<Vec<_>>(),
            ["A-First", "Z-Last"]
        );
        assert!(!application.is_dmg);
    }
}
