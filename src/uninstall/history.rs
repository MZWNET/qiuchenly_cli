use super::fmt_size;
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Serialize, Deserialize)]
pub(super) struct HistoryEntry {
    pub created_at: String,
    pub app: Option<String>,
    pub deleted_count: usize,
    pub failed_count: usize,
    pub skipped_count: usize,
    pub freed_bytes: u64,
}

impl HistoryEntry {
    pub fn new(
        app: Option<&str>,
        deleted_count: usize,
        failed_count: usize,
        freed_bytes: u64,
    ) -> Self {
        Self {
            created_at: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            app: app.map(String::from),
            deleted_count,
            failed_count,
            skipped_count: 0,
            freed_bytes,
        }
    }
}

fn load() -> Result<Vec<HistoryEntry>> {
    let path = crate::config::config_dir()?.join("uninstall-history.json");
    match fs::read_to_string(path) {
        Ok(raw) => Ok(serde_json::from_str(&raw)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn append(entry: HistoryEntry) -> Result<()> {
    let mut history = load()?;
    history.insert(0, entry);
    history.truncate(100);
    let dir = crate::config::config_dir()?;
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("uninstall-history.json"),
        serde_json::to_string_pretty(&history)?,
    )?;
    Ok(())
}

pub fn show_history(offset: u64, limit: u64) -> Result<()> {
    let history = load()?;
    let mut entries = history
        .iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(usize::try_from(limit).unwrap_or(usize::MAX))
        .peekable();
    if entries.peek().is_none() {
        println!("暂无卸载历史");
    }
    for entry in entries {
        println!(
            "{}  {}  删除 {}  失败 {}  释放 {}",
            entry.created_at,
            entry.app.as_deref().unwrap_or("-"),
            entry.deleted_count,
            entry.failed_count,
            fmt_size(entry.freed_bytes)
        );
    }
    Ok(())
}
