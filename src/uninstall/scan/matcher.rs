use crate::rules::Filter;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum NodeKind {
    File,
    Folder,
    Package,
}

pub(super) fn node_kind(path: &Path, metadata: &std::fs::Metadata) -> NodeKind {
    if !metadata.is_dir() {
        return NodeKind::File;
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("app" | "kext" | "prefpane" | "xpc" | "bundle" | "plugin" | "docset") => {
            NodeKind::Package
        }
        _ => NodeKind::Folder,
    }
}

pub(super) fn type_matches(filter: &Filter, kind: NodeKind) -> bool {
    match filter.r#type.as_deref() {
        None | Some("any") => true,
        Some("file") => kind == NodeKind::File,
        Some("folder") => kind == NodeKind::Folder,
        Some("package") => kind == NodeKind::Package,
        _ => true,
    }
}

pub(super) fn name_matches(filter: &Filter, name: &str) -> bool {
    filter.name.as_ref().is_none_or(|want| name == want)
        && filter
            .name_prefix
            .as_ref()
            .is_none_or(|prefix| name.starts_with(prefix.as_str()))
        && filter
            .name_suffix
            .as_ref()
            .is_none_or(|suffix| name.ends_with(suffix.as_str()))
}

pub(super) fn glob_match_any(patterns: &[String], value: &str) -> bool {
    patterns.iter().any(|pattern| {
        pattern
            .strip_suffix('*')
            .map_or(pattern == value, |prefix| value.starts_with(prefix))
    })
}

pub(super) fn ancestor_at_depth(path: &Path, depth: usize) -> PathBuf {
    path.components()
        .take(depth)
        .fold(PathBuf::from("/"), |path, component| {
            path.join(component.as_os_str())
        })
}

pub(super) fn infer_domain(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let extension_len = [
        ".plist",
        ".savedstate",
        ".xpc",
        ".app",
        ".kext",
        ".prefpane",
        ".dylib",
    ]
    .iter()
    .find(|extension| lower.ends_with(**extension))
    .map_or(0, |extension| extension.len());
    ["com.", "org.", "io.", "net.", "cn.", "dev.", "me.", "app."]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        .then(|| name[..name.len() - extension_len].to_string())
}

pub(in crate::uninstall) fn infer_domain_from_path(path: &Path) -> Option<String> {
    infer_domain(&path.file_name()?.to_string_lossy())
}
