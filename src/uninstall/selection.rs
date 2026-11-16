use super::scan::ScanItem;
use anyhow::Result;
use dialoguer::MultiSelect;

pub(super) fn select<'a>(items: &[&'a ScanItem], yes: bool) -> Result<Vec<&'a ScanItem>> {
    if yes {
        return Ok(items
            .iter()
            .copied()
            .filter(|item| {
                item.deletable && item.confidence == "high" && item.shared_with.is_empty()
            })
            .collect());
    }
    let labels: Vec<String> = items
        .iter()
        .map(|item| {
            format!(
                "{} [{}]{}",
                item.path.display(),
                item.confidence,
                if item.shared_with.is_empty() {
                    ""
                } else {
                    " [共享]"
                }
            )
        })
        .collect();
    let selected = MultiSelect::new()
        .with_prompt("选择要删除的项目")
        .items(&labels)
        .interact()?;
    Ok(selected.into_iter().map(|index| items[index]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn yes_selects_only_deletable_high_confidence_unshared_items() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("item");
        fs::write(&path, "data").unwrap();
        let high = ScanItem::app(path).unwrap();
        let mut medium = high.clone();
        medium.confidence = "medium";
        let mut shared = high.clone();
        shared.shared_with.push("com.example".into());
        let mut protected = high.clone();
        protected.deletable = false;
        let selected = select(&[&high, &medium, &shared, &protected], true).unwrap();
        assert_eq!(selected.len(), 1);
        assert!(std::ptr::eq(selected[0], &high));
    }
}
