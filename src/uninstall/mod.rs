mod delete;
mod history;
mod scan;
mod selection;

use crate::macos::apps;
use crate::rules::source::{expand_home, AppContext, SourceRuleType};
use crate::rules::{self, RuleDoc, RuleEntry};
use anyhow::{bail, Context, Result};
use scan::{infer_domain_from_path, ScanItem, Scanner};
use std::collections::HashSet;
use std::path::PathBuf;

pub use history::show_history;

pub fn uninstall_app(target: &str, yes: bool, dry_run: bool) -> Result<()> {
    let app_path = if target.ends_with(".app") {
        let path = PathBuf::from(target);
        if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        }
    } else {
        apps::find_installed_app(target)
            .map_err(|_| anyhow::anyhow!("本地未安装 {target}，或直接传 .app 路径"))?
            .path
    };
    if !app_path.exists() {
        bail!("应用路径不存在: {}", app_path.display());
    }
    let app = apps::parse_app_bundle(&app_path)?;
    let doc = rules::load_rules()?;
    let mut context = AppContext::from_installed(&app);
    context.load_team_id(&app_path);
    check_official_uninstaller(&doc, &app.bundle_id, &app.name)?;
    check_source_uninstaller(&doc, &context)?;
    println!("扫描 {} ...", app_path.display());
    let mut scanner = Scanner::new(doc)?;
    scanner.scan_app(&context);
    scanner.items.retain(|item| {
        if item.from_source {
            return true;
        }
        let domain = infer_domain_from_path(&item.path);
        domain
            .as_deref()
            .is_some_and(|d| d.eq_ignore_ascii_case(&app.bundle_id))
            || (domain.is_none()
                && item
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(&app.name)))
    });
    scanner.items.insert(0, ScanItem::app(app_path)?);
    scanner.post_process(false);
    finish_scan(scanner.items, yes, dry_run, Some(&app.bundle_id))
}

pub fn scan_orphans(yes: bool, dry_run: bool) -> Result<()> {
    println!("扫描孤儿文件 ...");
    let mut scanner = Scanner::new(rules::load_rules()?)?;
    let installed_apps = apps::scan_installed_apps()?;
    let installed: HashSet<String> = installed_apps
        .iter()
        .map(|app| app.bundle_id.to_lowercase())
        .collect();
    let installed_contexts: Vec<AppContext> = installed_apps
        .iter()
        .map(AppContext::from_installed)
        .collect();
    scanner.scan_orphans(&installed_contexts);
    let mut common: HashSet<String> = scanner
        .doc
        .protection
        .common_words
        .iter()
        .map(|word| word.to_lowercase())
        .collect();
    common.extend(scanner.protection.words.iter().cloned());
    scanner.items.retain(|item| {
        let name = item
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if common.contains(&name) {
            return false;
        }
        if item.from_source {
            return infer_domain_from_path(&item.path)
                .is_none_or(|domain| !installed.contains(&domain.to_lowercase()));
        }
        infer_domain_from_path(&item.path)
            .is_some_and(|domain| !installed.contains(&domain.to_lowercase()))
    });
    scanner.post_process(true);
    finish_scan(scanner.items, yes, dry_run, None)
}

pub fn leftovers(app: &str, yes: bool, dry_run: bool) -> Result<()> {
    println!("按规则扫描 {app} 的残留 ...");
    let mut scanner = Scanner::new(rules::load_rules()?)?;
    scanner.scan_bundle(app);
    scanner.items.retain(|item| {
        item.from_source
            || infer_domain_from_path(&item.path)
                .is_some_and(|domain| domain.eq_ignore_ascii_case(app))
    });
    if scanner.items.is_empty() {
        println!("没有发现残留");
        return Ok(());
    }
    scanner.post_process(true);
    finish_scan(scanner.items, yes, dry_run, Some(app))
}

fn finish_scan(items: Vec<ScanItem>, yes: bool, dry_run: bool, app: Option<&str>) -> Result<()> {
    let deletable: Vec<&ScanItem> = items.iter().filter(|item| item.deletable).collect();
    let total: u64 = deletable.iter().filter_map(|item| item.size_bytes).sum();
    println!(
        "发现 {} 项（{} 可删），约 {}",
        items.len(),
        deletable.len(),
        fmt_size(total)
    );
    for item in &items {
        let size = item.size_bytes.map(fmt_size).unwrap_or_else(|| "-".into());
        let shared = if item.shared_with.is_empty() {
            String::new()
        } else {
            format!("  [共享: {}]", item.shared_with.join(", "))
        };
        println!(
            "  [{:<4}] {:<6} {:>9} {}{}",
            if item.deletable { "可删" } else { "保留" },
            item.confidence,
            size,
            item.path.display(),
            shared
        );
        if let Some(reason) = &item.reason {
            println!("          ↳ {reason}");
        }
    }
    if dry_run {
        println!("（dry-run，未删除）");
        return Ok(());
    }
    if deletable.is_empty() {
        println!("没有可删项");
        return Ok(());
    }
    let chosen = selection::select(&deletable, yes)?;
    if chosen.is_empty() {
        println!("未选择任何项，退出");
        return Ok(());
    }
    let mut protected: Vec<PathBuf> = rules::load_rules()?
        .protection
        .skip_paths
        .iter()
        .map(|path| rules::expand_path(path))
        .collect();
    protected.extend(protect_paths_for_delete()?);
    let mut deleted = 0;
    let mut failed = Vec::new();
    let mut freed = 0;
    println!("删除 {} 项...", chosen.len());
    for item in chosen {
        match delete::delete(item, &protected) {
            Ok(()) => {
                deleted += 1;
                freed += item.size_bytes.unwrap_or(0);
            }
            Err(error) => failed.push((item.path.clone(), format!("{error:#}"))),
        }
    }
    println!(
        "完成: 删除 {deleted} 项，失败 {}，释放 {}",
        failed.len(),
        fmt_size(freed)
    );
    for (path, reason) in &failed {
        println!("  失败: {} {}", path.display(), reason);
    }
    history::append(history::HistoryEntry::new(
        app,
        deleted,
        failed.len(),
        freed,
    ))
    .context("保存卸载历史失败，删除操作可能已经完成")?;

    if !failed.is_empty() {
        bail!("{} 项删除失败", failed.len());
    }

    Ok(())
}

/// 删除前的第二道防线：protect 规则里不带通配的路径直接挡掉
fn protect_paths_for_delete() -> Result<Vec<PathBuf>> {
    let doc = rules::load_rules()?;
    let mut out = Vec::new();
    for entry in &doc.rules {
        let RuleEntry::Source(rule) = entry else {
            continue;
        };
        if rule.kind != SourceRuleType::Protect {
            continue;
        }
        for item in rule.items.iter().filter(|item| item.enabled) {
            let Some(path) = &item.path else {
                continue;
            };
            if path.contains('*') || path.contains('?') {
                continue;
            }
            let expanded = expand_placeholders_for_delete(path);
            if let Some(path) = expanded {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn expand_placeholders_for_delete(path: &str) -> Option<PathBuf> {
    expand_home(path).ok()
}

fn check_official_uninstaller(doc: &RuleDoc, bundle_id: &str, name: &str) -> Result<()> {
    for official in &doc.official_uninstallers {
        if official
            .bundle_prefixes
            .iter()
            .any(|prefix| bundle_id.starts_with(prefix.trim_end_matches('*')))
            || official
                .name_fragments
                .iter()
                .any(|fragment| name.to_lowercase().contains(&fragment.to_lowercase()))
        {
            match &official.reason {
                Some(reason) => bail!("{}", reason),
                None => bail!("请使用 {} 官方卸载器，避免破坏共享组件", official.vendor),
            }
        }
    }
    Ok(())
}

/// 匹配目标的源码规则声明了官方卸载器时，像旧 officialUninstallers 一样阻止
fn check_source_uninstaller(doc: &RuleDoc, context: &AppContext) -> Result<()> {
    for entry in &doc.rules {
        let RuleEntry::Source(rule) = entry else {
            continue;
        };
        let Some(uninstaller) = &rule.uninstaller else {
            continue;
        };
        if rule.matches_app(context) {
            bail!(
                "请先使用 {} 官方卸载器: {}",
                uninstaller.name,
                uninstaller.what
            );
        }
    }
    Ok(())
}

fn fmt_size(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (1 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MiB", bytes as f64 / (1 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KiB", bytes as f64 / (1 << 10) as f64)
    } else {
        format!("{bytes} B")
    }
}
