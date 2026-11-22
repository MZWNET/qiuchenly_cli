use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub(super) struct AppConfig {
    #[serde(rename = "packageName")]
    pub package_name: PackageName,
    #[serde(default, rename = "appBaseLocate")]
    pub app_base_locate: Option<String>,
    #[serde(default, rename = "bridgeFile")]
    pub bridge_file: Option<String>,
    #[serde(default, rename = "injectFile")]
    pub inject_file: Option<String>,
    #[serde(default, rename = "supportVersion")]
    support_version: Option<Vec<String>>,
    #[serde(default, rename = "supportSubVersion")]
    support_sub_version: Option<Vec<String>>,
    #[serde(default, rename = "needCopyToAppDir")]
    pub need_copy_to_app_dir: Option<bool>,
    #[serde(default, rename = "deepSignApp")]
    pub deep_sign_app: Option<bool>,
    #[serde(default, rename = "noSignTarget")]
    pub no_sign_target: Option<bool>,
    #[serde(default, rename = "disableLibraryValidate")]
    pub disable_library_validate: Option<bool>,
    #[serde(default)]
    pub entitlements: Option<String>,
    #[serde(default, rename = "dylibSelect")]
    pub dylib_select: Option<String>,
    #[serde(default, rename = "linkAbility")]
    pub link_ability: Option<String>,
    #[serde(default)]
    pub tccutil: Option<serde_json::Value>,
    #[serde(default, rename = "keygen")]
    pub keygen: Option<bool>,
}

impl AppConfig {
    pub fn matches(&self, bundle_id: &str) -> bool {
        self.package_name
            .patterns()
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => bundle_id.starts_with(prefix),
                None => bundle_id == pattern,
            })
    }

    pub fn allows_version(&self, version: &str) -> bool {
        let prefix_allowed = self.support_version.as_ref().is_none_or(|versions| {
            versions.is_empty() || versions.iter().any(|prefix| version.starts_with(prefix))
        });
        let suffix_allowed = self.support_sub_version.as_ref().is_none_or(|versions| {
            versions.is_empty() || versions.iter().any(|suffix| version.ends_with(suffix))
        });

        prefix_allowed && suffix_allowed
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum PackageName {
    One(String),
    Many(Vec<String>),
}

impl PackageName {
    pub fn patterns(&self) -> &[String] {
        match self {
            Self::One(pattern) => std::slice::from_ref(pattern),
            Self::Many(patterns) => patterns,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct InjectConfig {
    #[serde(default, rename = "AppList")]
    pub app_list: Vec<AppConfig>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct CachedInjectList {
    #[serde(rename = "storeVersion")]
    pub store_version: String,
    pub config: InjectConfig,
}

fn cached_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("inject-list.json"))
}

pub(super) fn load_cached() -> Result<Option<CachedInjectList>> {
    let path = cached_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read(&path).with_context(|| format!("读取 {}", path.display()))?;

    serde_json::from_slice(&data)
        .with_context(|| format!("解析 {} 失败", path.display()))
        .map(Some)
}

pub(super) fn save_cached(cached: &CachedInjectList) -> Result<()> {
    let path = cached_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("创建 {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(cached).context("序列化注入列表失败")?;
    fs::write(&path, data).with_context(|| format!("写入 {}", path.display()))
}

pub(super) fn load() -> Result<InjectConfig> {
    let override_path = std::env::var_os("HAYAKU_CONFIG").map(PathBuf::from);
    let default_paths = [
        Path::new("/Library/Application Support/Hayaku/config.json"),
        Path::new("/Library/Application Support/Hayaku/active/config.json"),
        Path::new(
            "/Library/Application Support/Hayaku/HayakuDaemon.app/Contents/Resources/config.json",
        ),
        Path::new(
            "/Applications/QiuChenAppStore.app/Contents/Resources/HayakuDaemon.app/Contents/Resources/config.json",
        ),
    ];
    for path in override_path.as_deref().into_iter().chain(default_paths) {
        if path.exists() {
            let data = fs::read(path).with_context(|| format!("读取 {}", path.display()))?;

            return serde_json::from_slice(&data)
                .with_context(|| format!("解析 {} 失败", path.display()));
        }
    }

    bail!("找不到注入配置；用 HAYAKU_CONFIG 指定 config.json 路径")
}

pub(super) fn find_dylib_source() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("HAYAKU_INJECT_DYLIB") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
    }
    for base in [
        "/Library/Application Support/Hayaku",
        "/Applications/QiuChenAppStore.app/Contents/Resources",
    ] {
        let path =
            PathBuf::from(base).join("HayakuDaemon.app/Contents/Resources/tool/CoreInject.dylib");
        if path.exists() {
            return Ok(path);
        }
    }

    bail!("找不到 CoreInject.dylib；用 HAYAKU_INJECT_DYLIB 指定注入库路径")
}
