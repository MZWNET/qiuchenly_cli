use serde::Deserialize;

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
