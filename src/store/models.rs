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

#[derive(Debug, Deserialize)]
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
    #[serde(rename = "releaseNotes", default)]
    pub release_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    #[serde(rename = "refreshToken")]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublicKey {
    #[serde(rename = "publicKey")]
    pub public_key: String,
}

#[derive(Debug, Deserialize)]
pub struct User {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct UserAppPackage {
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub kind: String,
    #[serde(default)]
    pub architecture: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserAppRelease {
    pub id: String,
    pub version: String,
    #[serde(rename = "fullVersion", default)]
    pub full_version: String,
    #[serde(default)]
    pub beta: bool,
    #[serde(default)]
    pub packages: Vec<UserAppPackage>,
}

#[derive(Debug, Deserialize)]
pub struct UserAppDownload {
    pub url: String,
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
}
