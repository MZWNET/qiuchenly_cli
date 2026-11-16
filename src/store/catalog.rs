use super::client::Client;
use super::models::AppVersion;
use crate::macos::apps::scan_installed_apps;
use anyhow::{Context, Result};

fn select_bundle_id<'a>(
    app: &str,
    candidates: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    exact_only: bool,
) -> Result<Option<String>> {
    let exact = exact_only
        || candidates
            .clone()
            .any(|(name, _)| name.eq_ignore_ascii_case(app));
    let mut matches = candidates.filter(|(name, _)| !exact || name.eq_ignore_ascii_case(app));
    let Some((_, bundle_id)) = matches.next() else {
        return Ok(None);
    };
    if let Some((_, second)) = matches.next() {
        let ids = std::iter::once(bundle_id)
            .chain(std::iter::once(second))
            .chain(matches.map(|(_, id)| id))
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!("应用名称 {app} 不明确，请指定 bundle ID：{ids}");
    }

    Ok(Some(bundle_id.to_string()))
}

pub async fn resolve_bundle_id(client: &Client, app: &str) -> Result<String> {
    if app.contains('.') && !app.contains(' ') {
        return Ok(app.to_string());
    }
    let local = scan_installed_apps()?;
    let candidates = local
        .iter()
        .map(|item| (item.name.as_str(), item.bundle_id.as_str()));
    if let Some(bundle_id) = select_bundle_id(app, candidates, true)? {
        return Ok(bundle_id);
    }
    let remote = client.search(app, 1, 20).await?;
    let candidates = remote
        .list
        .iter()
        .map(|item| (item.name.as_str(), item.bundle_id.as_str()));

    select_bundle_id(app, candidates, false)?.with_context(|| format!("找不到应用：{app}"))
}

fn select_version(versions: Vec<AppVersion>, requested: Option<&str>) -> Option<AppVersion> {
    versions.into_iter().find(|version| {
        requested.is_none_or(|requested| {
            version.version == requested || version.full_version == requested
        })
    })
}

pub async fn resolve_version(
    client: &Client,
    app: &str,
    version: Option<&str>,
) -> Result<(String, AppVersion)> {
    let bundle_id = resolve_bundle_id(client, app).await?;
    let versions = client.app_versions(&bundle_id).await?;
    let selected = select_version(versions, version).with_context(|| match version {
        Some(version) => format!("{bundle_id} 没有版本 {version}"),
        None => format!("{bundle_id} 没有可用版本"),
    })?;

    Ok((bundle_id, selected))
}

pub fn is_newer(installed: &str, available: &str) -> Result<bool> {
    let ordering = version_compare::compare(available, installed)
        .map_err(|_| anyhow::anyhow!("无法比较版本：{installed} 和 {available}"))?;

    Ok(ordering == version_compare::Cmp::Gt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_names_require_bundle_id() {
        let candidates = [("App One", "one.id"), ("App Two", "two.id")];
        let error = select_bundle_id("App", candidates.iter().copied(), false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("one.id") && error.contains("two.id"));
        assert_eq!(
            select_bundle_id("app one", candidates.iter().copied(), false)
                .unwrap()
                .as_deref(),
            Some("one.id")
        );
        assert!(select_bundle_id("App", candidates.iter().copied(), true)
            .unwrap()
            .is_none());
        let duplicates = [("App", "one.id"), ("app", "two.id")];
        assert!(select_bundle_id("APP", duplicates.iter().copied(), true).is_err());
        assert_eq!(
            select_bundle_id("App", candidates[..1].iter().copied(), false)
                .unwrap()
                .as_deref(),
            Some("one.id")
        );
    }

    #[test]
    fn version_selection_preserves_order_and_requires_requested_version() {
        let versions = || {
            vec![
                AppVersion {
                    version: "1.0".into(),
                    full_version: "1.0.1".into(),
                    url: "first".into(),
                    beta: false,
                },
                AppVersion {
                    version: "2.0".into(),
                    full_version: "2.0.1".into(),
                    url: "second".into(),
                    beta: false,
                },
            ]
        };
        assert_eq!(select_version(versions(), None).unwrap().url, "first");
        assert_eq!(
            select_version(versions(), Some("2.0.1")).unwrap().url,
            "second"
        );
        assert!(select_version(versions(), Some("3.0")).is_none());
    }

    #[test]
    fn only_strictly_newer_versions_are_updates() {
        assert!(is_newer("1.0", "1.1").unwrap());
        assert!(!is_newer("1.1", "1.0").unwrap());
        assert!(!is_newer("1.0", "1.0").unwrap());
        assert!(is_newer("unknown", "1.0").is_err());
    }
}
