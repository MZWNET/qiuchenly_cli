use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
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

#[derive(Deserialize)]
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

#[derive(Deserialize)]
pub(super) struct InjectConfig {
    #[serde(default, rename = "AppList")]
    pub app_list: Vec<AppConfig>,
}

pub(super) fn load() -> Result<InjectConfig> {
    let override_path = std::env::var_os("HAYAKU_CONFIG").map(PathBuf::from);
    let default_path = Path::new("/Library/Application Support/Hayaku/config.json");
    for path in override_path
        .as_deref()
        .into_iter()
        .chain(std::iter::once(default_path))
    {
        if path.exists() {
            let data = fs::read(path).with_context(|| format!("读取 {}", path.display()))?;

            return serde_json::from_slice(&data)
                .with_context(|| format!("解析 {} 失败", path.display()));
        }
    }

    bail!(
        "找不到注入配置（HAYAKU_CONFIG={override_path:?}，默认 {}）",
        default_path.display()
    )
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
