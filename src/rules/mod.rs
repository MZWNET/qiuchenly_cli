use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 与原版 1.4.11 uninstall-rules/hayaku.json 同构的规则文档。
/// 内置于二进制，云端可用 /uninstall-rules/compose 订阅合集覆盖。
const DEFAULT_RULES: &str = include_str!("hayaku.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDoc {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i64,
    pub version: String,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub domains: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub protection: Protection,
    #[serde(default, rename = "officialUninstallers")]
    pub official_uninstallers: Vec<OfficialUninstaller>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Protection {
    #[serde(default, rename = "skipPaths")]
    pub skip_paths: Vec<String>,
    #[serde(default, rename = "dataProtectedBundles")]
    pub data_protected_bundles: Vec<String>,
    #[serde(default, rename = "commonWords")]
    pub common_words: Vec<String>,
    #[serde(default, rename = "skipExtensions")]
    pub skip_extensions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialUninstaller {
    pub vendor: String,
    #[serde(default, rename = "bundlePrefixes")]
    pub bundle_prefixes: Vec<String>,
    #[serde(default, rename = "nameFragments")]
    pub name_fragments: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub path: String,
    #[serde(default)]
    pub deep: i64,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub deep: i64,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "namePrefix")]
    pub name_prefix: Option<String>,
    #[serde(default, rename = "nameSuffix")]
    pub name_suffix: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub separate: Option<bool>,
    #[serde(default, rename = "goDeeper")]
    pub go_deeper: Option<bool>,
    #[serde(default, rename = "isSystem")]
    pub is_system: Option<bool>,
    #[serde(default, rename = "hideInRemains")]
    pub hide_in_remains: Option<bool>,
}

fn rules_path() -> Result<PathBuf> {
    Ok(crate::config::config_dir()?.join("uninstall-rules.json"))
}

pub fn load_rules() -> Result<RuleDoc> {
    let p = rules_path()?;
    if p.exists() {
        let raw = fs::read_to_string(&p)?;
        let doc: RuleDoc =
            serde_json::from_str(&raw).with_context(|| format!("解析 {} 失败", p.display()))?;
        return Ok(doc);
    }
    Ok(serde_json::from_str(DEFAULT_RULES)?)
}

/// 展开路径里的 ~ 与已知的伪变量
pub fn expand_path(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        return home.join(rest.trim_start_matches('/'));
    }
    PathBuf::from(p)
}

fn save_rules(doc: &RuleDoc, origin: &str) -> Result<()> {
    let p = rules_path()?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = serde_json::to_value(doc)?;
    if let Some(obj) = out.as_object_mut() {
        obj.insert("origin".into(), serde_json::json!(origin));
    }
    fs::write(&p, serde_json::to_string_pretty(&out)?)?;
    println!("规则已保存: {} (version {})", p.display(), doc.version);
    Ok(())
}

/// 从云端拉订阅合集并合成规则文档（原版 syncToLocal 流程）
pub async fn sync() -> Result<()> {
    let client = crate::store::client::Client::authenticated()?;
    let raw = client
        .get_json("/uninstall-rules/subscriptions/me", &[])
        .await?;
    let subs: Vec<serde_json::Value> = match raw {
        serde_json::Value::Array(a) => a,
        v => vec![v],
    };
    if subs.is_empty() {
        println!("云端没有已订阅的规则合集，保留本地规则");
        return Ok(());
    }
    if subs.len() > 50 {
        anyhow::bail!("订阅合集超过 50 个上限（{}）", subs.len());
    }
    let ids: Vec<String> = subs
        .iter()
        .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();
    let composed: serde_json::Value = client
        .post(
            "/uninstall-rules/compose",
            &serde_json::json!({ "collectionIds": ids }),
        )
        .await?;
    let doc: RuleDoc = serde_json::from_value(
        composed
            .get("document")
            .cloned()
            .context("合成结果缺少 document")?,
    )?;
    save_rules(&doc, "server")
}

pub fn show() -> Result<()> {
    let doc = load_rules()?;
    println!(
        "规则版本 {}（{} 条根规则，{} 个保护路径）",
        doc.version,
        doc.rules.len(),
        doc.protection.skip_paths.len()
    );
    for rule in doc.rules.iter().take(8) {
        println!(
            "  {} (deep={}, filters={})",
            rule.path,
            rule.deep,
            rule.filters.len()
        );
    }
    if doc.rules.len() > 8 {
        println!("  ... 共 {} 条", doc.rules.len());
    }
    Ok(())
}

pub fn reset() -> Result<()> {
    let doc: RuleDoc = serde_json::from_str(DEFAULT_RULES)?;
    save_rules(&doc, "builtin")
}
