use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub mod source;

use crate::rules::source::SourceRule;

/// 与原版 1.4.11 uninstall-rules/hayaku.json 同构的规则文档。
/// 内置于二进制，云端可用 /uninstall-rules/compose 订阅合集覆盖，
/// 合集里也可以是线上源码 DSL 规则（只有 version/rules）。
const DEFAULT_RULES: &str = include_str!("hayaku.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDoc {
    #[serde(
        default,
        rename = "schemaVersion",
        skip_serializing_if = "Option::is_none"
    )]
    pub schema_version: Option<i64>,
    pub version: String,
    #[serde(default)]
    pub rules: Vec<RuleEntry>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub domains: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Protection::is_empty")]
    pub protection: Protection,
    #[serde(
        default,
        rename = "officialUninstallers",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub official_uninstallers: Vec<OfficialUninstaller>,
}

/// 旧编译规则与线上源码规则共用 rules 数组，按字段形状区分
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuleEntry {
    Compiled(Rule),
    Source(Box<SourceRule>),
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

impl Protection {
    fn is_empty(&self) -> bool {
        self.skip_paths.is_empty()
            && self.data_protected_bundles.is_empty()
            && self.common_words.is_empty()
            && self.skip_extensions.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialUninstaller {
    pub vendor: String,
    #[serde(default, rename = "bundlePrefixes")]
    pub bundle_prefixes: Vec<String>,
    #[serde(default, rename = "nameFragments")]
    pub name_fragments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub path: String,
    #[serde(default = "default_rule_deep")]
    pub deep: i64,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default = "default_filter_deep")]
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

fn default_rule_deep() -> i64 {
    1
}

fn default_filter_deep() -> i64 {
    -1
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
        doc.validate()?;
        return Ok(doc);
    }
    let doc: RuleDoc = serde_json::from_str(DEFAULT_RULES)?;
    doc.validate()?;
    Ok(doc)
}

impl RuleDoc {
    pub fn validate(&self) -> Result<()> {
        for entry in &self.rules {
            if let RuleEntry::Source(rule) = entry {
                rule.validate()
                    .with_context(|| format!("规则「{}」未通过校验", rule.title))?;
            }
        }
        Ok(())
    }
}

/// 展开路径里的 ~ 与已知的伪变量
pub fn expand_path(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        return home.join(rest.trim_start_matches('/'));
    }
    PathBuf::from(p)
}

fn save_document(mut value: serde_json::Value, version: &str, origin: &str) -> Result<()> {
    let p = rules_path()?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("origin".into(), serde_json::json!(origin));
    }
    fs::write(&p, serde_json::to_string_pretty(&value)?)?;
    println!("规则已保存: {} (version {})", p.display(), version);
    Ok(())
}

/// 从云端拉订阅合集并合成规则文档（原版 syncToLocal 流程）
pub async fn sync() -> Result<()> {
    let client = crate::store::client::Client::authenticated()?;
    let subs = client.uninstall_rule_subscriptions().await?;
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
    let document = client.compose_uninstall_rules(&ids).await?;
    let doc: RuleDoc = serde_json::from_str(&document).context("解析合成的规则文档失败")?;
    doc.validate().context("合成的规则文档未通过校验")?;
    save_document(serde_json::from_str(&document)?, &doc.version, "server")
}

pub fn show() -> Result<()> {
    let doc = load_rules()?;
    let compiled = doc
        .rules
        .iter()
        .filter(|rule| matches!(rule, RuleEntry::Compiled(_)))
        .count();
    let source = doc.rules.len() - compiled;
    println!(
        "规则版本 {}（{} 条编译规则，{} 条源码规则，{} 个保护路径）",
        doc.version,
        compiled,
        source,
        doc.protection.skip_paths.len()
    );
    for rule in doc.rules.iter().take(8) {
        match rule {
            RuleEntry::Compiled(rule) => println!(
                "  {} (deep={}, filters={})",
                rule.path,
                rule.deep,
                rule.filters.len()
            ),
            RuleEntry::Source(rule) => println!(
                "  [{}] {} (items={})",
                rule.kind_str(),
                rule.title,
                rule.items.len()
            ),
        }
    }
    if doc.rules.len() > 8 {
        println!("  ... 共 {} 条", doc.rules.len());
    }
    Ok(())
}

pub fn reset() -> Result<()> {
    let doc: RuleDoc = serde_json::from_str(DEFAULT_RULES)?;
    save_document(serde_json::to_value(&doc)?, &doc.version, "builtin")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_deep_defaults_to_rule_one_filter_minus_one() {
        let doc: RuleDoc = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "version": "test",
                "rules": [
                    {
                        "path": "/Applications",
                        "filters": [
                            { "nameSuffix": ".app" }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();
        let RuleEntry::Compiled(rule) = &doc.rules[0] else {
            panic!("应解析为编译规则");
        };
        assert_eq!(rule.deep, 1);
        assert_eq!(rule.filters[0].deep, -1);
    }

    #[test]
    fn official_uninstaller_reason_is_optional() {
        let doc: RuleDoc = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "version": "test",
                "officialUninstallers": [
                    {
                        "vendor": "ESET",
                        "bundlePrefixes": ["com.eset."]
                    }
                ]
            }"#,
        )
        .unwrap();
        assert!(doc.official_uninstallers[0].reason.is_none());
    }

    #[test]
    fn source_only_document_parses_and_round_trips() {
        let doc: RuleDoc = serde_json::from_str(
            r#"{
                "version": "2026.11.16",
                "rules": [
                    {
                        "type": "cache",
                        "title": "Tool 缓存",
                        "app": { "bundleId": "com.example.Tool" },
                        "items": [
                            { "path": "~/Library/Caches/{bundleId}", "what": "缓存目录" }
                        ]
                    }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(doc.schema_version, None);
        assert_eq!(doc.rules.len(), 1);
        assert!(matches!(doc.rules[0], RuleEntry::Source(_)));
        doc.validate().unwrap();
        let out = serde_json::to_value(&doc).unwrap();
        let obj = out.as_object().unwrap();
        assert!(!obj.contains_key("schemaVersion"));
        assert!(!obj.contains_key("domains"));
        assert!(!obj.contains_key("protection"));
        assert!(!obj.contains_key("officialUninstallers"));
        let reparsed: RuleDoc = serde_json::from_value(out).unwrap();
        assert!(matches!(reparsed.rules[0], RuleEntry::Source(_)));
    }

    #[test]
    fn builtin_hayaku_rules_still_parse() {
        let doc: RuleDoc = serde_json::from_str(DEFAULT_RULES).unwrap();
        assert_eq!(doc.schema_version, Some(1));
        assert!(!doc.rules.is_empty());
        assert!(doc
            .rules
            .iter()
            .all(|rule| matches!(rule, RuleEntry::Compiled(_))));
        doc.validate().unwrap();
        let out = serde_json::to_value(&doc).unwrap();
        assert_eq!(out["schemaVersion"], 1);
    }
}
