use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CatalogEntry {
    #[serde(rename = "seriesId")]
    pub series_id: String,
    #[serde(rename = "bundleId")]
    pub bundle_id: String,
    pub name: String,
    #[serde(default)]
    pub introduction: String,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub developer: Vec<String>,
    #[serde(default)]
    pub architecture: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CatalogList {
    #[serde(default)]
    pub list: Vec<CatalogEntry>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppVersion {
    pub version: String,
    #[serde(rename = "fullVersion", default)]
    pub full_version: String,
    pub url: String,
    #[serde(default)]
    pub beta: bool,
}

#[derive(Debug, Deserialize)]
pub struct Category {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CategoryApps {
    #[serde(default)]
    pub list: Vec<CategoryApp>,
    #[serde(default)]
    pub total: u64,
}

#[derive(Debug, Deserialize)]
pub struct CategoryApp {
    #[serde(rename = "bundleId")]
    pub bundle_id: String,
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(default)]
    pub rating: f64,
    #[serde(rename = "ratingCount", default)]
    pub rating_count: u64,
}

#[derive(Debug, Deserialize)]
pub struct PopularBind {
    #[serde(rename = "bundleId")]
    pub bundle_id: String,
    #[serde(rename = "appName")]
    pub app_name: String,
    pub version: String,
    #[serde(rename = "downloadCount", default)]
    pub download_count: u64,
}

#[derive(Deserialize)]
pub struct Popular {
    pub binds: Vec<PopularBind>,
}

#[derive(Debug, Deserialize)]
pub struct GuiUpdate {
    #[serde(rename = "hasUpdate")]
    pub has_update: bool,
    #[serde(rename = "latestPackage")]
    pub latest_package: Option<LatestPackage>,
}

#[derive(Debug, Deserialize)]
pub struct LatestPackage {
    pub version: String,
    #[serde(rename = "fullVersion", default)]
    pub full_version: String,
    #[serde(default)]
    pub beta: bool,
}

#[derive(Debug, Deserialize)]
pub struct GuiUpdateDownload {
    #[serde(rename = "downloadUrl")]
    pub download_url: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub sha256: String,
    pub version: String,
}
