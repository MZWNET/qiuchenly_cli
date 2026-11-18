use super::item::{Identity, ScanItem};
use super::matcher::{
    ancestor_at_depth, glob_match_any, infer_domain, infer_domain_from_path, name_matches,
    node_kind, type_matches,
};
use crate::rules::{self, RuleDoc};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(in crate::uninstall) struct Scanner {
    pub doc: RuleDoc,
    pub items: Vec<ScanItem>,
}

impl Scanner {
    pub fn new(doc: RuleDoc) -> Self {
        Self {
            doc,
            items: Vec::new(),
        }
    }

    pub fn scan(&mut self) {
        let skip_paths: HashSet<PathBuf> = self
            .doc
            .protection
            .skip_paths
            .iter()
            .map(|p| rules::expand_path(p))
            .collect();
        for rule in &self.doc.rules {
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
                .filter_entry(|entry| entry.depth() == 0 || !skip_paths.contains(entry.path()))
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
            item.size_bytes = item_size(&item.path);
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
