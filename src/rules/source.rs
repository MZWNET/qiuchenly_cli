use anyhow::{bail, Context, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRule {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: SourceRuleType,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub app: Option<SourceAppSelector>,
    #[serde(default, rename = "where")]
    pub where_: Option<SourceWhere>,
    #[serde(default)]
    pub depth: Option<u8>,
    pub items: Vec<SourceItem>,
    #[serde(default)]
    pub uninstaller: Option<SourceUninstaller>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRuleType {
    App,
    Cache,
    Junk,
    Logs,
    LoginItems,
    SystemExtensions,
    Protect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceAppSelector {
    All(String),
    App(SourceApp),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceApp {
    #[serde(default, rename = "bundleId")]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default, rename = "teamId")]
    pub team_id: Option<String>,
    #[serde(default)]
    pub family: Vec<String>,
    #[serde(default)]
    pub detect: Vec<AppDetect>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDetect {
    pub field: String,
    pub equals: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceWhere {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceItem {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub word: Option<String>,
    #[serde(default)]
    pub what: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub delete: Option<DeletePolicy>,
    #[serde(default, rename = "type")]
    pub kind: SourceItemType,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub scan: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeletePolicy {
    Yes,
    #[default]
    Ask,
    No,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceItemType {
    #[default]
    Any,
    File,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceUninstaller {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    pub what: String,
}

#[derive(Debug, Clone, Default)]
pub struct AppContext {
    pub bundle_id: Option<String>,
    pub names: Vec<String>,
    pub team_id: Option<String>,
    pub info: HashMap<String, String>,
}

impl AppContext {
    pub fn from_installed(app: &crate::macos::apps::InstalledApp) -> Self {
        let mut context = Self {
            bundle_id: Some(app.bundle_id.clone()),
            names: vec![app.name.clone()],
            ..Self::default()
        };
        if let Some(name) = app.path.file_stem().and_then(|name| name.to_str()) {
            context.push_name(name);
        }
        if let Ok(info) =
            plist::from_file::<_, plist::Dictionary>(app.path.join("Contents/Info.plist"))
        {
            for (key, value) in info {
                if let Some(value) = value.as_string() {
                    if matches!(key.as_str(), "CFBundleName" | "CFBundleDisplayName") {
                        context.push_name(value);
                    }
                    context.info.insert(key, value.to_string());
                }
            }
        }
        context
    }

    pub fn bundle_id(bundle_id: &str) -> Self {
        Self {
            bundle_id: Some(bundle_id.to_string()),
            ..Self::default()
        }
    }

    pub fn load_team_id(&mut self, app_path: &Path) {
        let Ok(output) = Command::new("codesign")
            .arg("-d")
            .arg("--verbose=4")
            .arg(app_path)
            .output()
        else {
            return;
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        self.team_id = stderr.lines().find_map(|line| {
            line.trim()
                .strip_prefix("TeamIdentifier=")
                .filter(|team| *team != "(not signed)")
                .map(str::to_string)
        });
    }

    fn from_declaration(app: &SourceApp) -> Self {
        Self {
            bundle_id: app.bundle_id.clone(),
            names: app.names.clone(),
            team_id: app.team_id.clone(),
            info: HashMap::new(),
        }
    }

    fn push_name(&mut self, name: &str) {
        if !self
            .names
            .iter()
            .any(|known| known.eq_ignore_ascii_case(name))
        {
            self.names.push(name.to_string());
        }
    }
}

impl SourceRule {
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            bail!("规则标题不能为空");
        }
        if self.items.is_empty() {
            bail!("规则「{}」没有条目", self.title);
        }
        if let Some(depth) = self.depth {
            if !(1..=4).contains(&depth) {
                bail!("规则「{}」depth 只能是 1 到 4", self.title);
            }
        }
        match self.kind {
            SourceRuleType::App if self.app.is_none() => {
                bail!("App 规则「{}」必须填写 app", self.title)
            }
            SourceRuleType::Protect => {
                if self.app.is_some() || self.where_.is_some() || self.depth.is_some() {
                    bail!("保护规则「{}」不能填写 app、where 或 depth", self.title);
                }
                if self.uninstaller.is_some() {
                    bail!("保护规则「{}」不能填写 uninstaller", self.title);
                }
            }
            _ => {}
        }
        if let Some(SourceAppSelector::All(value)) = &self.app {
            if value != "*" {
                bail!("规则「{}」的 app 字符串只能是 *", self.title);
            }
        }
        if let Some(SourceAppSelector::App(app)) = &self.app {
            app.validate(&self.title)?;
        }
        if let Some(where_) = &self.where_ {
            for path in where_.as_slice() {
                validate_path(path, true)
                    .with_context(|| format!("规则「{}」where", self.title))?;
            }
        }
        if let Some(uninstaller) = &self.uninstaller {
            if self.app.is_none() {
                bail!("规则「{}」的官方卸载器缺少目标 app", self.title);
            }
            if let Some(path) = &uninstaller.path {
                validate_path(path, true)
                    .with_context(|| format!("规则「{}」uninstaller.path", self.title))?;
            }
        }
        for (index, item) in self.items.iter().enumerate() {
            self.validate_item(item)
                .with_context(|| format!("规则「{}」第 {} 项", self.title, index + 1))?;
        }
        Ok(())
    }

    fn validate_item(&self, item: &SourceItem) -> Result<()> {
        let matchers = [
            item.path.is_some(),
            item.name.is_some(),
            item.regex.is_some(),
            item.bundle.is_some(),
            item.word.is_some(),
        ];
        if matchers.into_iter().filter(|present| *present).count() != 1 {
            bail!("必须且只能填写 path、name、regex、bundle、word 中的一种");
        }
        if self.kind == SourceRuleType::Protect {
            if item.name.is_some() || item.regex.is_some() {
                bail!("保护规则只能使用 path、bundle 或 word");
            }
            if item.why.as_deref().is_none_or(|why| why.trim().is_empty()) {
                bail!("保护规则必须填写 why");
            }
            if item.what.is_some() || item.delete.is_some() {
                bail!("保护规则不能填写 what 或 delete");
            }
            if item.kind != SourceItemType::Any {
                bail!("暂不支持保护条目的 type 限制");
            }
        } else {
            if item.bundle.is_some() || item.word.is_some() || item.why.is_some() {
                bail!("非保护规则不能使用 bundle、word 或 why");
            }
            if item
                .what
                .as_deref()
                .is_none_or(|what| what.trim().is_empty())
            {
                bail!("清理条目必须填写 what");
            }
            if item.scan.is_some() {
                bail!("scan 只能用于保护规则");
            }
        }
        if let Some(path) = &item.path {
            validate_path(path, self.kind == SourceRuleType::Protect)?;
            if self.kind == SourceRuleType::Protect && path.contains('{') {
                bail!("保护规则没有目标 App，路径不能含占位符");
            }
            if has_glob(path) && !path.contains('{') {
                compile_path_glob(path)?;
            }
        }
        if let Some(regex) = &item.regex {
            if regex.chars().count() > 256 {
                bail!("regex 最长 256 个字符");
            }
            compile_regex(regex)?;
        }
        if let Some(bundle) = &item.bundle {
            compile_glob(bundle)?;
        }
        if let Some(name) = &item.name {
            if name.is_empty() {
                bail!("name 不能为空");
            }
            compile_glob(name)?;
        }
        if item
            .word
            .as_deref()
            .is_some_and(|word| word.trim().is_empty())
        {
            bail!("word 不能为空");
        }
        Ok(())
    }

    pub fn default_where(&self) -> &'static [&'static str] {
        match self.kind {
            SourceRuleType::App | SourceRuleType::Protect => &[],
            SourceRuleType::Cache => &["~/Library/Caches"],
            SourceRuleType::Junk => &["~"],
            SourceRuleType::Logs => &["~/Library/Logs", "~/Library/Logs/DiagnosticReports"],
            SourceRuleType::LoginItems => &[
                "~/Library/LaunchAgents",
                "/Library/LaunchAgents",
                "/Library/LaunchDaemons",
            ],
            SourceRuleType::SystemExtensions => {
                &["/Library/SystemExtensions", "/Library/Extensions"]
            }
        }
    }

    pub fn default_depth(&self) -> usize {
        match self.kind {
            SourceRuleType::App
            | SourceRuleType::Cache
            | SourceRuleType::LoginItems
            | SourceRuleType::SystemExtensions => 1,
            SourceRuleType::Junk | SourceRuleType::Logs => 2,
            SourceRuleType::Protect => 1,
        }
    }

    pub fn default_delete(&self) -> DeletePolicy {
        match self.kind {
            SourceRuleType::App | SourceRuleType::Cache | SourceRuleType::Logs => DeletePolicy::Yes,
            SourceRuleType::Junk
            | SourceRuleType::LoginItems
            | SourceRuleType::SystemExtensions => DeletePolicy::Ask,
            SourceRuleType::Protect => DeletePolicy::No,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            SourceRuleType::App => "app",
            SourceRuleType::Cache => "cache",
            SourceRuleType::Junk => "junk",
            SourceRuleType::Logs => "logs",
            SourceRuleType::LoginItems => "login-items",
            SourceRuleType::SystemExtensions => "system-extensions",
            SourceRuleType::Protect => "protect",
        }
    }

    pub fn matches_app(&self, app: &AppContext) -> bool {
        match &self.app {
            Some(SourceAppSelector::All(_)) => true,
            Some(SourceAppSelector::App(expected)) => expected.matches(app),
            None => false,
        }
    }

    pub fn declaration_context(&self) -> Option<AppContext> {
        match &self.app {
            Some(SourceAppSelector::App(app)) => Some(AppContext::from_declaration(app)),
            _ => None,
        }
    }

    pub fn target_context(&self, actual: &AppContext) -> Option<AppContext> {
        match &self.app {
            Some(SourceAppSelector::All(_)) => Some(actual.clone()),
            Some(SourceAppSelector::App(expected)) if expected.matches(actual) => {
                let mut context = AppContext::from_declaration(expected);
                if context.bundle_id.is_none() {
                    context.bundle_id = actual.bundle_id.clone();
                }
                if context.team_id.is_none() {
                    context.team_id = actual.team_id.clone();
                }
                if context.names.is_empty() {
                    context.names = actual.names.clone();
                }
                Some(context)
            }
            _ => None,
        }
    }

    pub fn is_installed_for_rule(&self, installed: &[AppContext]) -> bool {
        let Some(SourceAppSelector::App(expected)) = &self.app else {
            return false;
        };
        installed.iter().any(|actual| {
            expected.matches(actual)
                || actual.bundle_id.as_deref().is_some_and(|bundle_id| {
                    expected
                        .family
                        .iter()
                        .any(|family| family.eq_ignore_ascii_case(bundle_id))
                })
        })
    }
}

impl SourceWhere {
    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::One(path) => std::slice::from_ref(path),
            Self::Many(paths) => paths,
        }
    }
}

impl SourceApp {
    fn validate(&self, title: &str) -> Result<()> {
        if self.bundle_id.is_none() && self.names.is_empty() && self.detect.is_empty() {
            bail!("规则「{title}」的 app 至少需要 bundleId、names 或 detect");
        }
        if let Some(bundle_id) = &self.bundle_id {
            validate_bundle_id(bundle_id, title)?;
        }
        for family in &self.family {
            validate_bundle_id(family, title)?;
        }
        if self.names.iter().any(|name| name.is_empty()) {
            bail!("规则「{title}」的 app.names 不能包含空名字");
        }
        if self.team_id.as_deref().is_some_and(|team| {
            team.len() != 10
                || !team
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        }) {
            bail!("规则「{title}」的 teamId 必须是 10 位大写字母或数字");
        }
        if self.detect.iter().any(|detect| detect.field.is_empty()) {
            bail!("规则「{title}」的 detect.field 不能为空");
        }
        if self
            .detect
            .iter()
            .any(|detect| !matches!(detect.field.as_str(), "CFBundleIdentifier" | "CFBundleName"))
        {
            bail!("规则「{title}」的 detect 只支持 CFBundleIdentifier 和 CFBundleName");
        }
        Ok(())
    }

    fn matches(&self, actual: &AppContext) -> bool {
        if self.bundle_id.as_deref().is_some_and(|expected| {
            actual
                .bundle_id
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        }) {
            return true;
        }
        if self.detect.iter().any(|detect| {
            actual
                .info
                .get(&detect.field)
                .is_some_and(|actual| actual == &detect.equals)
                || (detect.field == "CFBundleIdentifier"
                    && actual.bundle_id.as_deref() == Some(detect.equals.as_str()))
        }) {
            return true;
        }
        if self.family.iter().any(|family| {
            actual
                .bundle_id
                .as_deref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(family))
        }) {
            return true;
        }
        self.names.iter().any(|expected| {
            actual
                .names
                .iter()
                .any(|actual| actual.eq_ignore_ascii_case(expected))
        })
    }
}

#[derive(Clone)]
pub struct ProtectionPath {
    pub path: PathBuf,
    pub recursive: bool,
    pub scan: bool,
    pub reason: String,
    glob: Option<globset::GlobMatcher>,
}

impl ProtectionPath {
    fn source(raw: &str, scan: bool, reason: &str) -> Result<Self> {
        let recursive = raw.len() > 1 && raw.ends_with('/');
        let path = expand_home(raw.trim_end_matches('/'))?;
        let path = if path.as_os_str().is_empty() {
            PathBuf::from("/")
        } else {
            path
        };
        let glob = path
            .to_str()
            .filter(|path| path.contains('*') || path.contains('?'))
            .map(compile_path_glob)
            .transpose()?;
        Ok(Self {
            path,
            recursive,
            scan,
            reason: reason.to_string(),
            glob,
        })
    }

    pub fn matches(&self, path: &Path) -> bool {
        if self.glob.is_some() {
            return path.ancestors().any(|candidate| {
                (candidate == path || self.recursive) && self.direct_match(candidate)
            });
        }
        path == self.path || (self.recursive && path.starts_with(&self.path))
    }

    pub fn direct_match(&self, path: &Path) -> bool {
        self.glob.as_ref().map_or(path == self.path, |glob| {
            path.to_str().is_some_and(|path| glob.is_match(path))
        })
    }

    pub fn has_glob(&self) -> bool {
        self.glob.is_some()
    }
}

#[derive(Clone, Default)]
pub struct SourceProtection {
    pub paths: Vec<ProtectionPath>,
    bundles: Vec<(Regex, String)>,
    pub words: Vec<String>,
}

impl SourceProtection {
    pub fn compile(rules: &[crate::rules::RuleEntry]) -> Result<Self> {
        let mut protection = Self::default();
        for rule in rules {
            let crate::rules::RuleEntry::Source(rule) = rule else {
                continue;
            };
            if rule.kind != SourceRuleType::Protect {
                continue;
            }
            for item in rule.items.iter().filter(|item| item.enabled) {
                let reason = item.why.clone().unwrap_or_default();
                if let Some(path) = &item.path {
                    protection.paths.push(ProtectionPath::source(
                        path,
                        item.scan.unwrap_or(true),
                        &reason,
                    )?);
                }
                if let Some(bundle) = &item.bundle {
                    protection.bundles.push((compile_glob(bundle)?, reason));
                }
                if let Some(word) = &item.word {
                    protection.words.push(word.to_lowercase());
                }
            }
        }
        Ok(protection)
    }

    pub fn path_reason(&self, path: &Path) -> Option<&str> {
        self.paths
            .iter()
            .find(|protected| protected.matches(path))
            .map(|protected| protected.reason.as_str())
    }

    pub fn bundle_reason(&self, bundle_id: &str) -> Option<&str> {
        self.bundles
            .iter()
            .find(|(pattern, _)| pattern.is_match(bundle_id))
            .map(|(_, reason)| reason.as_str())
    }
}

pub fn compile_glob(pattern: &str) -> Result<Regex> {
    let mut source = String::from("^");
    let mut literal = String::new();
    for character in pattern.chars() {
        match character {
            '*' | '?' => {
                if !literal.is_empty() {
                    source.push_str(&regex::escape(&literal));
                    literal.clear();
                }
                source.push_str(if character == '*' { ".*" } else { "." });
            }
            _ => literal.push(character),
        }
    }
    if !literal.is_empty() {
        source.push_str(&regex::escape(&literal));
    }
    source.push('$');
    RegexBuilder::new(&source)
        .case_insensitive(true)
        .dot_matches_new_line(true)
        .build()
        .with_context(|| format!("无效通配符 {pattern}"))
}

pub fn compile_regex(pattern: &str) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .with_context(|| format!("无效正则 {pattern}"))
}

/// 路径通配用 globset，区分大小写，* 可跨 /
pub fn compile_path_glob(pattern: &str) -> Result<globset::GlobMatcher> {
    Ok(globset::GlobBuilder::new(pattern)
        .literal_separator(false)
        .build()
        .with_context(|| format!("无效路径通配 {pattern}"))?
        .compile_matcher())
}

pub fn has_glob(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?')
}

pub fn expand_placeholders(pattern: &str, app: &AppContext) -> Result<Vec<String>> {
    let Some(start) = pattern.find('{') else {
        return Ok(vec![pattern.to_string()]);
    };
    let end = pattern[start..]
        .find('}')
        .map(|end| start + end)
        .context("路径占位符缺少 }")?;
    let placeholder = &pattern[start + 1..end];
    let values: Vec<&str> = match placeholder {
        "bundleId" => app.bundle_id.as_deref().into_iter().collect(),
        "name" => app.names.iter().map(String::as_str).collect(),
        "teamId" => app.team_id.as_deref().into_iter().collect(),
        _ => bail!("不支持的占位符 {{{placeholder}}}"),
    };
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let mut expanded = Vec::new();
    for value in values {
        let candidate = format!("{}{}{}", &pattern[..start], value, &pattern[end + 1..]);
        expanded.extend(expand_placeholders(&candidate, app)?);
    }
    Ok(expanded)
}

pub fn expand_home(path: &str) -> Result<PathBuf> {
    if path == "~" || path.starts_with("~/") {
        let home = dirs::home_dir().context("无法确定用户主目录")?;
        return Ok(home.join(path.strip_prefix("~/").unwrap_or("")));
    }
    Ok(PathBuf::from(path))
}

fn validate_path(path: &str, allow_root: bool) -> Result<()> {
    if !(path == "~" || path.starts_with("~/") || path.starts_with('/')) {
        bail!("路径必须以 ~ 或 / 开头: {path}");
    }
    if path.contains("..") {
        bail!("路径不能包含 ..: {path}");
    }
    if !allow_root && matches!(path.trim_end_matches('/'), "" | "~") {
        bail!("清理路径不能是整个磁盘或用户主目录");
    }
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let end = rest[start..]
            .find('}')
            .map(|end| start + end)
            .context("占位符缺少 }")?;
        if !matches!(&rest[start + 1..end], "bundleId" | "name" | "teamId") {
            bail!("不支持的占位符 {{{}}}", &rest[start + 1..end]);
        }
        rest = &rest[end + 1..];
    }
    Ok(())
}

fn validate_bundle_id(bundle_id: &str, title: &str) -> Result<()> {
    if bundle_id.is_empty()
        || !bundle_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("规则「{title}」包含无效包名 {bundle_id}");
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_expand_to_stable_alternatives() {
        let app = AppContext {
            bundle_id: Some("com.example.Tool".into()),
            names: vec!["Tool".into(), "Tool Old".into()],
            team_id: Some("ABCDE12345".into()),
            info: HashMap::new(),
        };
        let paths = expand_placeholders(
            "~/Library/Group Containers/{teamId}.{bundleId}/{name}",
            &app,
        )
        .unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].ends_with("ABCDE12345.com.example.Tool/Tool"));
        assert!(paths[1].ends_with("ABCDE12345.com.example.Tool/Tool Old"));
        assert!(
            expand_placeholders("/Library/{teamId}", &AppContext::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn glob_is_case_insensitive_and_anchored() {
        let exact = compile_glob("com.example.Tool").unwrap();
        assert!(exact.is_match("COM.EXAMPLE.tool"));
        assert!(!exact.is_match("com.example.Toolbox"));
        let glob = compile_glob("log?.txt").unwrap();
        assert!(glob.is_match("LOG1.TXT"));
        assert!(!glob.is_match("log12.txt"));
    }

    #[test]
    fn source_rule_rejects_unknown_or_widening_fields() {
        assert!(serde_json::from_str::<SourceRule>(
            r#"{"type":"cache","title":"Cache","items":[{"name":"*","what":"all","unknown":true}]}"#,
        )
        .is_err());
        let rule: SourceRule = serde_json::from_str(
            r#"{"type":"cache","title":"Cache","items":[{"name":"*","what":"all","scan":false}]}"#,
        )
        .unwrap();
        assert!(rule.validate().is_err());
    }

    #[test]
    fn family_member_matches_declared_app() {
        let rule: SourceRule = serde_json::from_str(
            r#"{
                "type": "app",
                "title": "套件",
                "app": { "bundleId": "com.main.app", "family": ["com.member.app"] },
                "items": [{ "path": "/Applications/Main.app", "what": "本体" }]
            }"#,
        )
        .unwrap();
        rule.validate().unwrap();
        let member = AppContext::bundle_id("com.member.app");
        assert!(rule.matches_app(&member));
        assert!(rule.target_context(&member).is_some());
        assert!(rule.is_installed_for_rule(&[member]));
        let stranger = AppContext::bundle_id("com.other.app");
        assert!(!rule.matches_app(&stranger));
        assert!(rule.target_context(&stranger).is_none());
    }

    #[test]
    fn target_context_fills_placeholders_from_actual_when_declared_gaps() {
        let rule: SourceRule = serde_json::from_str(
            r#"{
                "type": "app",
                "title": "按名字认",
                "app": { "names": ["Old Tool"] },
                "items": [{ "path": "~/Library/Caches/{bundleId}", "what": "缓存" }]
            }"#,
        )
        .unwrap();
        rule.validate().unwrap();
        let mut actual = AppContext::bundle_id("com.actual.tool");
        actual.push_name("Old Tool");
        let context = rule.target_context(&actual).unwrap();
        let paths = expand_placeholders("~/Library/Caches/{bundleId}", &context).unwrap();
        assert_eq!(paths, vec!["~/Library/Caches/com.actual.tool".to_string()]);
    }

    #[test]
    fn detect_only_supports_known_info_plist_fields() {
        let rule: SourceRule = serde_json::from_str(
            r#"{
                "type": "app",
                "title": "老应用",
                "app": { "detect": [{ "field": "LSUIElement", "equals": "1" }] },
                "items": [{ "path": "/Applications/Old.app", "what": "本体" }]
            }"#,
        )
        .unwrap();
        assert!(rule.validate().is_err());
        let mut info = HashMap::new();
        info.insert("CFBundleName".to_string(), "Old Tool".to_string());
        let good: SourceRule = serde_json::from_str(
            r#"{
                "type": "app",
                "title": "老应用",
                "app": { "detect": [{ "field": "CFBundleName", "equals": "Old Tool" }] },
                "items": [{ "path": "/Applications/Old.app", "what": "本体" }]
            }"#,
        )
        .unwrap();
        good.validate().unwrap();
        let actual = AppContext {
            info,
            ..AppContext::default()
        };
        assert!(good.matches_app(&actual));
    }
}
