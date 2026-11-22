use super::item::{Identity, ScanItem};
use super::walk::walk_tree;
use crate::rules::source::{
    compile_glob, compile_path_glob, compile_regex, expand_home, expand_placeholders, has_glob,
    AppContext, DeletePolicy, ProtectionPath, SourceItem, SourceItemType, SourceProtection,
    SourceRule,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::DirEntry;

/// scan=false 的保护路径不作为候选，扫描时跳过
pub(in crate::uninstall) struct SkipPaths {
    exact: HashSet<PathBuf>,
    globs: Vec<ProtectionPath>,
}

impl SkipPaths {
    pub fn new(protection: &SourceProtection) -> Self {
        let mut skip = Self {
            exact: HashSet::new(),
            globs: Vec::new(),
        };
        for protected in &protection.paths {
            if protected.scan {
                continue;
            }
            if protected.has_glob() {
                skip.globs.push(protected.clone());
            } else {
                skip.exact.insert(protected.path.clone());
            }
        }
        skip
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.exact.contains(path) || self.globs.iter().any(|glob| glob.direct_match(path))
    }
}

pub(super) fn scan_source_rule(
    rule: &SourceRule,
    context: &AppContext,
    skip: &SkipPaths,
    out: &mut Vec<ScanItem>,
) {
    for item in rule.items.iter().filter(|item| item.enabled) {
        if let Some(pattern) = &item.path {
            scan_path_item(rule, item, pattern, context, skip, out);
        } else {
            scan_where_item(rule, item, context, skip, out);
        }
    }
}

/// path 条目不经过 where/depth，无通配直接看目标本身
fn scan_path_item(
    rule: &SourceRule,
    item: &SourceItem,
    pattern: &str,
    context: &AppContext,
    skip: &SkipPaths,
    out: &mut Vec<ScanItem>,
) {
    let Ok(expanded) = expand_placeholders(pattern, context) else {
        return;
    };
    for raw in expanded {
        let base = match expand_home(&raw) {
            Ok(base) => base,
            Err(_) => continue,
        };
        let candidates: Vec<PathBuf> = if has_glob(&raw) {
            glob_paths(&base, false, skip)
        } else if fs::symlink_metadata(&base).is_ok() {
            vec![base]
        } else {
            Vec::new()
        };
        for path in candidates {
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !type_ok(item.kind, &metadata) || skip.contains(&path) {
                continue;
            }
            push_item(out, rule, item, path, &metadata);
        }
    }
}

/// name/regex 条目按 where（或类型默认位置）走 WalkDir
fn scan_where_item(
    rule: &SourceRule,
    item: &SourceItem,
    context: &AppContext,
    skip: &SkipPaths,
    out: &mut Vec<ScanItem>,
) {
    let roots: Vec<String> = match &rule.where_ {
        Some(where_) => where_.as_slice().to_vec(),
        None => rule.default_where().iter().map(|p| p.to_string()).collect(),
    };
    let depth = rule.depth.unwrap_or_else(|| rule.default_depth() as u8) as usize;
    let name_glob = item
        .name
        .as_deref()
        .map(|pattern| compile_glob(pattern).expect("validate 已保证 name 有效"));
    let name_regex = item
        .regex
        .as_deref()
        .map(|pattern| compile_regex(pattern).expect("validate 已保证 regex 有效"));
    for root in &roots {
        let Ok(expanded) = expand_placeholders(root, context) else {
            continue;
        };
        for raw in expanded {
            let base = match expand_home(&raw) {
                Ok(base) => base,
                Err(_) => continue,
            };
            for dir in resolve_roots(&base, skip) {
                let is_skipped = |path: &Path| skip.contains(path);
                let visit = |entry: &DirEntry| {
                    let path = entry.path();
                    let Ok(metadata) = fs::symlink_metadata(path) else {
                        return false;
                    };
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    let matched = name_glob.as_ref().is_some_and(|glob| glob.is_match(&name))
                        || name_regex
                            .as_ref()
                            .is_some_and(|regex| regex.is_match(&name));
                    if !matched || !type_ok(item.kind, &metadata) {
                        return false;
                    }
                    push_item(out, rule, item, path.to_path_buf(), &metadata);
                    metadata.is_dir()
                };
                walk_tree(&dir, Some(depth), is_skipped, visit);
            }
        }
    }
}

fn type_ok(kind: SourceItemType, metadata: &fs::Metadata) -> bool {
    match kind {
        SourceItemType::Any => true,
        SourceItemType::File => !metadata.is_dir(),
        SourceItemType::Folder => metadata.is_dir(),
    }
}

fn push_item(
    out: &mut Vec<ScanItem>,
    rule: &SourceRule,
    item: &SourceItem,
    path: PathBuf,
    metadata: &fs::Metadata,
) {
    let policy = item.delete.unwrap_or_else(|| rule.default_delete());
    let (deletable, confidence) = match policy {
        DeletePolicy::Yes => (true, "high"),
        DeletePolicy::Ask => (true, "medium"),
        DeletePolicy::No => (false, "high"),
    };
    let reason = match policy {
        DeletePolicy::No => item.why.clone().or_else(|| item.what.clone()),
        _ => item.what.clone(),
    };
    out.push(ScanItem {
        path,
        deletable,
        confidence,
        size_bytes: None,
        reason,
        shared_with: Vec::new(),
        identity: Identity::from_metadata(metadata),
        from_source: true,
    });
}

/// where 根目录：无通配要求是已存在目录，有通配枚举命中的目录
fn resolve_roots(base: &Path, skip: &SkipPaths) -> Vec<PathBuf> {
    if !has_glob(&base.to_string_lossy()) {
        return if fs::symlink_metadata(base).is_ok_and(|m| m.is_dir()) {
            vec![base.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    glob_paths(base, true, skip)
}

/// 从首个通配符前的静态父目录遍历，不跟 symlink，匹配完整绝对路径
fn glob_paths(expanded: &Path, dirs_only: bool, skip: &SkipPaths) -> Vec<PathBuf> {
    let Some(raw) = expanded.to_str() else {
        return Vec::new();
    };
    let Ok(matcher) = compile_path_glob(raw) else {
        return Vec::new();
    };
    let components: Vec<_> = expanded.components().collect();
    let Some(wildcard_at) = components.iter().position(|component| {
        let name = component.as_os_str().to_string_lossy();
        has_glob(&name)
    }) else {
        return Vec::new();
    };
    let mut root = PathBuf::new();
    for component in &components[..wildcard_at] {
        root.push(component.as_os_str());
    }
    if !fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    // path 里的 * 可跨 /，不限层级；命中目录后也不再深入
    let is_skipped = |path: &Path| skip.contains(path);
    let visit = |entry: &DirEntry| {
        let path = entry.path();
        let Some(raw) = path.to_str() else {
            return false;
        };
        if !matcher.is_match(raw) {
            return false;
        }
        if dirs_only && !entry.file_type().is_dir() {
            return false;
        }
        out.push(path.to_path_buf());
        entry.file_type().is_dir()
    };
    walk_tree(&root, None, is_skipped, visit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::RuleDoc;
    use crate::uninstall::{check_source_uninstaller, scan::Scanner};
    use std::fs;

    fn doc_with(rules_json: &str) -> RuleDoc {
        let json = format!(r#"{{"version":"test","rules":[{{{rules_json}}}]}}"#);
        serde_json::from_str(&json).unwrap_or_else(|error| panic!("{error}\n{json}"))
    }

    fn scanner_with(rules_json: &str) -> Scanner {
        let doc = doc_with(rules_json);
        doc.validate().unwrap();
        Scanner::new(doc).unwrap()
    }

    #[test]
    fn path_item_uses_target_placeholders_and_exact_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("com.example.Tool");
        fs::create_dir_all(&cache).unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}/{{bundleId}}","what":"缓存"}}]"#,
            temp.path().display()
        );
        let mut scanner = scanner_with(&rules);
        let app = AppContext::bundle_id("com.example.Tool");
        scanner.scan_target(&app);
        assert_eq!(scanner.items.len(), 1);
        assert!(scanner.items[0].from_source);
        assert!(scanner.items[0].deletable);
        assert_eq!(scanner.items[0].confidence, "high");
        // 目标不匹配的 App 不扫
        let mut other = scanner_with(&rules);
        other.scan_target(&AppContext::bundle_id("com.other.App"));
        assert!(other.items.is_empty());
    }

    #[test]
    fn path_glob_walks_static_parent_and_matches_full_path() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Library/Caches/Tool")).unwrap();
        fs::create_dir_all(temp.path().join("Library/Preferences/Tool")).unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}/Library/*/Tool","what":"残留"}}]"#,
            temp.path().display()
        );
        let mut scanner = scanner_with(&rules);
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        let paths: HashSet<PathBuf> = scanner.items.iter().map(|item| item.path.clone()).collect();
        let expected = HashSet::from([
            temp.path().join("Library/Caches/Tool"),
            temp.path().join("Library/Preferences/Tool"),
        ]);
        assert_eq!(paths, expected);
    }

    #[test]
    fn path_glob_star_crosses_multiple_directories() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Utilities/Parallels Tool.app")).unwrap();
        fs::create_dir_all(temp.path().join("Nested/Deep/Parallels Tool.app")).unwrap();
        let rules = format!(
            r#""type":"app","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}/*/Parallels Tool.app","what":"本体"}}]"#,
            temp.path().display()
        );
        let mut scanner = scanner_with(&rules);
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        assert_eq!(scanner.items.len(), 2);
    }

    #[test]
    fn where_name_respects_depth_and_stops_at_matched_dir() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Caches");
        fs::create_dir_all(root.join("Tool/deep/inner")).unwrap();
        fs::write(root.join("Tool/deep/inner/Tool"), "x").unwrap();
        fs::write(root.join("shallow.Tool"), "x").unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "where":["{}"],"depth":2,
                "items":[{{"name":"*tool*","what":"缓存","type":"file"}}]"#,
            root.display()
        );
        let mut scanner = scanner_with(&rules);
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        assert_eq!(scanner.items.len(), 1);
        assert!(scanner.items[0].path.ends_with("shallow.Tool"));
        // depth=2 只到第二层，inner/Tool 不扫；目录命中后也不再深入
    }

    #[test]
    fn where_scan_false_protect_dir_prunes_descendants() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Keep/inner")).unwrap();
        fs::write(temp.path().join("Keep/keep.log"), "x").unwrap();
        fs::write(temp.path().join("Keep/inner/keep.log"), "x").unwrap();
        fs::write(temp.path().join("keep.log"), "x").unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "where":["{}"],"depth":3,
                "items":[{{"name":"keep.log","what":"缓存","type":"file"}}]"#,
            temp.path().display()
        );
        let protect = format!(
            r#""type":"protect","title":"p","items":[{{"path":"{}","why":"用户数据","scan":false}}]"#,
            temp.path().join("Keep").display()
        );
        let mut scanner = scanner_with(&format!("{rules}}},{{{protect}"));
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        let paths: Vec<PathBuf> = scanner.items.iter().map(|item| item.path.clone()).collect();
        assert_eq!(paths, vec![temp.path().join("keep.log")]);
    }

    #[test]
    fn path_glob_scan_false_protect_prunes_descendants() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Library/A1/Tool.app")).unwrap();
        fs::create_dir_all(temp.path().join("Library/B2/Tool.app")).unwrap();
        let rules = format!(
            r#""type":"app","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}/Library/*/Tool.app","what":"本体"}}]"#,
            temp.path().display()
        );
        let protect = format!(
            r#""type":"protect","title":"p","items":[{{"path":"{}/Library/A*","why":"用户数据","scan":false}}]"#,
            temp.path().display()
        );
        let mut scanner = scanner_with(&format!("{rules}}},{{{protect}"));
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        let paths: HashSet<PathBuf> = scanner.items.iter().map(|item| item.path.clone()).collect();
        assert_eq!(
            paths,
            HashSet::from([temp.path().join("Library/B2/Tool.app")])
        );
    }

    #[test]
    fn compiled_scan_honors_global_skip_paths_and_source_skip_glob() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("Caches/SkipMe/Tool")).unwrap();
        fs::create_dir_all(temp.path().join("Caches/Guarded/Tool")).unwrap();
        fs::create_dir_all(temp.path().join("Caches/Plain/Tool")).unwrap();
        let json = format!(
            r#"{{"version":"test",
                "protection":{{"skipPaths":["{}/Caches/SkipMe"]}},
                "rules":[
                    {{"path":"{}","deep":2,"filters":[{{"name":"Tool"}}]}},
                    {{"type":"protect","title":"p","items":[{{"path":"{}/Caches/Guard*","why":"用户数据","scan":false}}]}}
                ]}}"#,
            temp.path().display(),
            temp.path().join("Caches").display(),
            temp.path().display()
        );
        let doc: RuleDoc = serde_json::from_str(&json).unwrap();
        doc.validate().unwrap();
        let mut scanner = Scanner::new(doc).unwrap();
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        let paths: HashSet<PathBuf> = scanner.items.iter().map(|item| item.path.clone()).collect();
        assert_eq!(
            paths,
            HashSet::from([temp.path().join("Caches/Plain/Tool")])
        );
    }

    #[test]
    fn compiled_scan_root_in_skip_paths_still_matches_at_depth_zero() {
        let temp = tempfile::tempdir().unwrap();
        let caches = temp.path().join("Caches");
        fs::create_dir_all(&caches).unwrap();
        let json = format!(
            r#"{{"version":"test",
                "protection":{{"skipPaths":["{}"]}},
                "rules":[{{"path":"{}","deep":0,"filters":[{{"deep":0,"name":"Caches"}}]}}]}}"#,
            caches.display(),
            caches.display()
        );
        let doc: RuleDoc = serde_json::from_str(&json).unwrap();
        doc.validate().unwrap();
        let mut scanner = Scanner::new(doc).unwrap();
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        let paths: Vec<PathBuf> = scanner.items.iter().map(|item| item.path.clone()).collect();
        assert_eq!(paths, vec![caches]);
    }

    #[test]
    fn protect_path_overrides_yes_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("Keep");
        fs::create_dir_all(&target).unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}","what":"缓存","delete":"yes"}}]"#,
            target.display()
        );
        let protect = format!(
            r#""type":"protect","title":"p","items":[{{"path":"{}","why":"用户数据"}}]"#,
            target.display()
        );
        let mut scanner = scanner_with(&format!("{rules}}},{{{protect}"));
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        assert_eq!(scanner.items.len(), 1);
        scanner.post_process(false);
        assert!(!scanner.items[0].deletable);
        assert_eq!(scanner.items[0].reason.as_deref(), Some("用户数据"));
    }

    #[test]
    fn duplicate_paths_merge_conservatively_and_fold_children() {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("Shared");
        let child = shared.join("inner");
        fs::create_dir_all(&child).unwrap();
        let yes = format!(
            r#""type":"cache","title":"yes","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}","what":"a","delete":"yes"}}]"#,
            shared.display()
        );
        let ask = format!(
            r#""type":"cache","title":"ask","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}","what":"b","delete":"ask"}}]"#,
            shared.display()
        );
        let no = format!(
            r#""type":"cache","title":"no","app":{{"bundleId":"com.example.Tool"}},
                "items":[{{"path":"{}","what":"c","delete":"no"}}]"#,
            child.display()
        );
        let mut scanner = scanner_with(&format!("{yes}}},{{{ask}}},{{{no}"));
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        assert_eq!(scanner.items.len(), 3);
        scanner.post_process(false);
        // 同路径合并成一项，子项折叠进父项，no 最保守
        assert_eq!(scanner.items.len(), 1);
        assert_eq!(scanner.items[0].path, shared);
        assert!(!scanner.items[0].deletable);
        assert_eq!(scanner.items[0].reason.as_deref(), Some("c"));
    }

    #[test]
    fn sibling_prefix_paths_are_not_folded() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("Tool");
        let toolbox = temp.path().join("Toolbox");
        fs::create_dir_all(&tool).unwrap();
        fs::create_dir_all(&toolbox).unwrap();
        let rules = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.example.Tool"}},
                "items":[
                    {{"path":"{}","what":"a","delete":"yes"}},
                    {{"path":"{}","what":"b","delete":"ask"}}
                ]"#,
            tool.display(),
            toolbox.display()
        );
        let mut scanner = scanner_with(&rules);
        scanner.scan_target(&AppContext::bundle_id("com.example.Tool"));
        assert_eq!(scanner.items.len(), 2);
        scanner.post_process(false);
        assert_eq!(scanner.items.len(), 2);
    }

    #[test]
    fn orphans_scan_declared_apps_only() {
        let temp = tempfile::tempdir().unwrap();
        let leftover = temp.path().join("com.gone.app");
        fs::create_dir_all(&leftover).unwrap();
        let declared = format!(
            r#""type":"cache","title":"t","app":{{"bundleId":"com.gone.app"}},
                "items":[{{"path":"{}","what":"残留"}}]"#,
            leftover.display()
        );
        let wildcard = format!(
            r#""type":"cache","title":"w","app":"*",
                "items":[{{"path":"{}","what":"x"}}]"#,
            leftover.display()
        );
        let mut scanner = scanner_with(&format!("{declared}}},{{{wildcard}"));
        let installed = [AppContext::bundle_id("com.other.app")];
        scanner.scan_orphans(&installed);
        assert_eq!(scanner.items.len(), 1);
        // 该 App 已安装时跳过
        let mut installed_scanner = scanner_with(&declared);
        installed_scanner.scan_orphans(&[AppContext::bundle_id("com.gone.app")]);
        assert!(installed_scanner.items.is_empty());
    }

    #[test]
    fn orphans_scan_global_rules_without_app() {
        let temp = tempfile::tempdir().unwrap();
        let junk = temp.path().join("Trash.log");
        fs::write(&junk, "x").unwrap();
        let rules = format!(
            r#""type":"junk","title":"全局垃圾",
                "where":["{}"],
                "items":[{{"name":"trash*","what":"垃圾日志","type":"file"}}]"#,
            temp.path().display()
        );
        let mut scanner = scanner_with(&rules);
        scanner.scan_orphans(&[]);
        assert_eq!(scanner.items.len(), 1);
        assert!(scanner.items[0].path.ends_with("Trash.log"));
        // junk 默认 ask
        assert!(scanner.items[0].deletable);
        assert_eq!(scanner.items[0].confidence, "medium");
    }

    #[test]
    fn source_uninstaller_blocks_matching_target() {
        let doc = doc_with(
            r#""type":"app","title":"t","app":{"bundleId":"com.example.Tool"},
                "uninstaller":{"name":"Tool Uninstaller","what":"先跑官方卸载器"},
                "items":[{"path":"/Applications/Tool.app","what":"本体"}]"#,
        );
        doc.validate().unwrap();
        let error =
            check_source_uninstaller(&doc, &AppContext::bundle_id("com.example.Tool")).unwrap_err();
        assert!(error.to_string().contains("Tool Uninstaller"));
        assert!(check_source_uninstaller(&doc, &AppContext::bundle_id("com.other.App")).is_ok());
    }
}
