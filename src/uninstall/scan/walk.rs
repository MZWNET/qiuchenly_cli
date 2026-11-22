use std::path::Path;
use walkdir::{DirEntry, WalkDir};

/// 统一遍历：不跟链接、可选最大深度、忽略读失败、不访问根目录本身。
/// is_skipped 命中的路径直接跳过，目录剪枝；visit 返回 true 视为命中，
/// 目录同样剪枝不再深入
pub(super) fn walk_tree(
    root: &Path,
    max_depth: Option<usize>,
    is_skipped: impl Fn(&Path) -> bool,
    mut visit: impl FnMut(&DirEntry) -> bool,
) {
    let mut walk = WalkDir::new(root).follow_links(false);
    if let Some(max_depth) = max_depth {
        walk = walk.max_depth(max_depth);
    }
    let iter = &mut walk.into_iter();
    while let Some(entry) = iter.next() {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        if is_skipped(path) {
            if entry.file_type().is_dir() {
                iter.skip_current_dir();
            }
            continue;
        }
        if visit(&entry) && entry.file_type().is_dir() {
            iter.skip_current_dir();
        }
    }
}
