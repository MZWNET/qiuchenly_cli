use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(super) struct Product {
    pub(super) sap_code: String,
    pub(super) display_name: String,
    pub(super) version: String,
    pub(super) base_version: String,
    pub(super) platform: String,
    pub(super) build_guid: String,
    pub(super) manifest_url: Option<String>,
    pub(super) cdn: String,
    pub(super) dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone)]
pub(super) struct Dependency {
    pub(super) sap_code: String,
    pub(super) base_version: String,
}

#[derive(Debug, Clone)]
pub(super) struct Application {
    pub(super) sap_code: String,
    pub(super) name: String,
    pub(super) family_name: Option<String>,
    pub(super) product_version: String,
    pub(super) base_version: String,
    pub(super) platform: String,
    pub(super) install_dir: String,
    pub(super) app_launch: String,
    pub(super) supported_languages: Vec<String>,
    pub(super) cdn: String,
    pub(super) packages: Vec<Package>,
    pub(super) deferred_packages: BTreeSet<String>,
    pub(super) is_dmg: bool,
}

#[derive(Debug, Clone)]
pub(super) struct Package {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) full_name: String,
    pub(super) path: String,
    pub(super) condition: Option<String>,
    pub(super) download_size: Option<u64>,
    pub(super) sha256: Option<String>,
    pub(super) install_sequence: u64,
    pub(super) alias_name: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct DownloadedPackage {
    pub(super) package: Package,
    pub(super) path: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct DownloadedProduct {
    pub(super) application: Application,
    pub(super) directory: PathBuf,
    pub(super) packages: Vec<DownloadedPackage>,
}

pub(super) fn architecture(platform: &str) -> &'static str {
    match platform {
        "macarm64" => "arm64",
        p if p.starts_with("osx10") => "x64",
        _ if cfg!(target_arch = "aarch64") => "arm64",
        _ => "x64",
    }
}

pub(super) fn platform_preference() -> &'static [&'static str] {
    if cfg!(target_arch = "aarch64") {
        &["macarm64", "macuniversal", "osx10-64", "osx10"]
    } else {
        &["macuniversal", "osx10-64", "osx10"]
    }
}
