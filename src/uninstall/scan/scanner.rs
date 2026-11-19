use super::item::{Identity, ScanItem};
use super::matcher::{
    ancestor_at_depth, glob_match_any, infer_domain, infer_domain_from_path, name_matches,
    node_kind, type_matches,
};
use super::source::{scan_source_rule, SkipPaths};
use crate::rules::source::{AppContext, SourceProtection};
use crate::rules::{self, RuleDoc, RuleEntry};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(in crate::uninstall) struct Scanner {
    pub doc: RuleDoc,
    pub items: Vec<ScanItem>,
    pub(in crate::uninstall) protection: SourceProtection,
    pub(in crate::uninstall) skip: SkipPaths,
    target: Option<AppContext>,
}

impl Scanner {
    pub fn new(doc: RuleDoc) -> Result<Self> {
        let protection = SourceProtection::compile(&doc.rules)?;
        let skip = SkipPaths::new(&protection);
        Ok(Self {
            doc,
            items: Vec::new(),
            protection,
            skip,
            target: None,
        })
    }

    /// 按目标 App 扫描：编译规则全量扫，源码规则只扫匹配目标的
    pub fn scan_app(&mut self, app: &AppContext) {
        self.target = Some(app.clone());
        self.scan_compiled();
        for entry in &self.doc.rules {
            let RuleEntry::Source(rule) = entry else {
                continue;
            };
            if let Some(context) = rule.target_context(app) {
                scan_source_rule(rule, &context, &self.skip, &mut self.items);
            }
        }
    }

    /// 只知道 bundleId 的残留扫描
    pub fn scan_bundle(&mut self, bundle_id: &str) {
        let context = AppContext::bundle_id(bundle_id);
        self.target = Some(context.clone());
        self.scan_compiled();
        for entry in &self.doc.rules {
            let RuleEntry::Source(rule) = entry else {
                continue;
            };
            if let Some(context) = rule.target_context(&context) {
                scan_source_rule(rule, &context, &self.skip, &mut self.items);
            }
        }
    }

    /// 孤儿扫描：只扫声明了明确 App 对象且该 App 未安装的源码规则
    pub fn scan_orphans(&mut self, installed: &[AppContext]) {
        self.target = None;
        self.scan_compiled();
        let fallback = AppContext::default();
        for entry in &self.doc.rules {
            let RuleEntry::Source(rule) = entry else {
                continue;
            };
            if matches!(
                rule.app,
                Some(crate::rules::source::SourceAppSelector::All(_))
            ) {
                continue;
            }
            if rule.app.is_some() {
                if rule.is_installed_for_rule(installed) {
                    continue;
                }
                if let Some(context) = rule.declaration_context() {
                    scan_source_rule(rule, &context, &self.skip, &mut self.items);
                }
            } else if rule.kind != crate::rules::source::SourceRuleType::Protect {
                // 全局 cache/junk/logs 等规则没有目标 App，占位缺值时展开自然跳过
                scan_source_rule(rule, &fallback, &self.skip, &mut self.items);
            }
        }
    }

    fn scan_compiled(&mut self) {
        let skip_paths: HashSet<PathBuf> = self
            .doc
            .protection
            .skip_paths
            .iter()
            .map(|p| rules::expand_path(p))
            .collect();
        for entry in &self.doc.rules {
            let RuleEntry::Compiled(rule) = entry else {
                continue;
            };
            let root = rules::expand_path(&rule.path);
            if !fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let mut walk = WalkDir::new(&root).follow_links(false);
            if rule.deep >= 0 {
                walk = walk.max_depth(rule.deep as usize);
            }
            for entry in walk
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || (!skip_paths.contains(entry.path()) && !self.skip.contains(entry.path()))
                })
                .flatten()
            {
                let path = entry.path();
                let Ok(metadata) = fs::symlink_metadata(path) else {
                    continue;
                };
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                let kind = node_kind(path, &metadata);
                let mut matched = false;
                let mut domain = None;
                let mut separate = None;
                let mut is_system = None;
                for filter in &rule.filters {
                    if (filter.deep >= 0 && filter.deep != entry.depth() as i64)
                        || !type_matches(filter, kind)
                        || !name_matches(filter, &name)
                    {
                        continue;
                    }
                    matched = true;
                    domain = filter.domain.as_ref();
                    separate = filter.separate;
                    is_system = filter.is_system;
                }
                if !matched || separate == Some(false) {
                    continue;
                }
                self.items.push(ScanItem {
                    path: path.to_path_buf(),
                    deletable: !is_system.unwrap_or(false),
                    confidence: if domain.is_some() || infer_domain(&name).is_some() {
                        "high"
                    } else {
                        "medium"
                    },
                    size_bytes: None,
                    reason: is_system.unwrap_or(false).then(|| "系统组件".to_string()),
                    shared_with: Vec::new(),
                    identity: Identity::from_metadata(&metadata),
                    from_source: false,
                });
            }
        }
    }

    pub fn post_process(&mut self, hide_system: bool) {
        if hide_system {
            self.items.retain(|item| {
                infer_domain_from_path(&item.path).is_none_or(|domain| {
                    !glob_match_any(&self.doc.protection.data_protected_bundles, &domain)
                })
            });
        }
        let mut roots: BTreeMap<String, HashSet<PathBuf>> = BTreeMap::new();
        for item in &self.items {
            if let Some(domain) = infer_domain_from_path(&item.path) {
                roots
                    .entry(domain)
                    .or_default()
                    .insert(ancestor_at_depth(&item.path, 3));
            }
        }
        for item in &mut self.items {
            if let Some(domain) = infer_domain_from_path(&item.path) {
                if roots.get(&domain).is_some_and(|paths| paths.len() > 1) {
                    item.shared_with.push(domain);
                }
            }
        }
        self.apply_source_protection();
        self.merge_conservative();
        for item in &mut self.items {
            item.size_bytes = item_size(&item.path);
        }
    }

    /// protect 规则最后覆盖所有候选：路径命中或 bundle 命中即不可删
    fn apply_source_protection(&mut self) {
        let target_bundle = self
            .target
            .as_ref()
            .and_then(|context| context.bundle_id.clone());
        for item in &mut self.items {
            if let Some(reason) = self.protection.path_reason(&item.path) {
                item.deletable = false;
                item.reason = Some(reason.to_string());
                continue;
            }
            let domain = infer_domain_from_path(&item.path).or(target_bundle.clone());
            if let Some(reason) = domain
                .as_deref()
                .and_then(|domain| self.protection.bundle_reason(domain))
            {
                item.deletable = false;
                item.reason = Some(reason.to_string());
            }
        }
    }

    /// 重复路径保守合并（no > ask > yes），父子同时命中折叠子项
    fn merge_conservative(&mut self) {
        let mut sorted = std::mem::take(&mut self.items);
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let mut merged: Vec<ScanItem> = Vec::new();
        for item in sorted {
            if let Some(last) = merged.last_mut().filter(|last| last.path == item.path) {
                merge_into(last, item);
            } else {
                merged.push(item);
            }
        }
        let mut kept: Vec<ScanItem> = Vec::new();
        for item in merged {
            match kept
                .iter()
                .rposition(|parent| is_ancestor(&parent.path, &item.path))
            {
                Some(index) => merge_into(&mut kept[index], item),
                None => kept.push(item),
            }
        }
        self.items = kept;
    }
}

fn is_ancestor(parent: &Path, child: &Path) -> bool {
    child.starts_with(parent)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Policy {
    No,
    Ask,
    Yes,
}

fn policy_of(item: &ScanItem) -> Policy {
    if !item.deletable {
        Policy::No
    } else if item.confidence == "high" {
        Policy::Yes
    } else {
        Policy::Ask
    }
}

fn merge_into(parent: &mut ScanItem, mut item: ScanItem) {
    if policy_of(&item) < policy_of(parent) {
        parent.deletable = item.deletable;
        parent.confidence = item.confidence;
        parent.reason = item.reason.take();
    }
    if parent.reason.is_none() {
        parent.reason = item.reason;
    }
    for domain in item.shared_with {
        if !parent.shared_with.contains(&domain) {
            parent.shared_with.push(domain);
        }
    }
}

fn item_size(path: &Path) -> Option<u64> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() {
        return Some(metadata.len());
    }
    WalkDir::new(path)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .try_fold(0u64, |total, entry| {
            let entry = entry.ok()?;
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            Some(total + if metadata.is_dir() { 0 } else { metadata.len() })
        })
}
