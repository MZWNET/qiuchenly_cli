mod models;

pub use models::{
    AppVersion, CatalogList, Category, CategoryApps, GuiUpdate, Popular, PopularBind,
};

use super::client::Client;
use crate::macos::apps::scan_installed_apps;
use anyhow::{Context, Result};

impl Client {
    pub async fn search(&self, keyword: &str, page: u32, size: u32) -> Result<CatalogList> {
        self.get(
            "/catalog/entries",
            &[
                ("page", page.to_string()),
                ("size", size.to_string()),
                ("search", keyword.to_string()),
            ],
        )
        .await
    }

    pub async fn categories(&self) -> Result<Vec<Category>> {
        self.get("/public/category-dictionary", &[]).await
    }

    pub async fn category_apps(
        &self,
        key: &str,
        page: u32,
        size: u32,
        search: Option<&str>,
    ) -> Result<CategoryApps> {
        let mut query = vec![("page", page.to_string()), ("size", size.to_string())];
        if let Some(search) = search {
            query.push(("search", search.to_string()));
        }

        self.get(&format!("/public/category-dictionary/{key}/apps"), &query)
            .await
    }

    pub async fn app_versions(&self, bundle_id: &str) -> Result<Vec<AppVersion>> {
        self.get(&format!("/appinfo/versions/{bundle_id}"), &[])
            .await
    }

    pub async fn popular(&self) -> Result<Vec<PopularBind>> {
        let data: Popular = self.get("/public/popular-downloads", &[]).await?;

        Ok(data.binds)
    }

    pub async fn check_gui_update(&self, version: &str) -> Result<GuiUpdate> {
        self.get(
            &format!("/public/checkGuiAppUpdate/{version}"),
            &[("updateCheckType", "beta".into())],
        )
        .await
    }
}

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

pub trait Release {
    fn version(&self) -> &str;
    fn full_version(&self) -> &str;
    fn beta(&self) -> bool;
}

pub fn select_release<'a, T: Release>(
    releases: &'a [T],
    requested: Option<&str>,
    allow_beta: bool,
) -> Option<&'a T> {
    match requested {
        Some(requested) => releases
            .iter()
            .find(|release| release.version() == requested || release.full_version() == requested),
        None if allow_beta => releases.first(),
        None => releases.iter().find(|release| !release.beta()),
    }
}

impl Release for AppVersion {
    fn version(&self) -> &str {
        &self.version
    }

    fn full_version(&self) -> &str {
        &self.full_version
    }

    fn beta(&self) -> bool {
        self.beta
    }
}

pub async fn resolve_version(
    client: &Client,
    app: &str,
    version: Option<&str>,
    beta: bool,
) -> Result<(String, AppVersion)> {
    let bundle_id = resolve_bundle_id(client, app).await?;
    let versions = client.app_versions(&bundle_id).await?;
    let selected = select_release(&versions, version, beta)
        .cloned()
        .with_context(|| match version {
            Some(version) => format!("{bundle_id} 没有版本 {version}"),
            None if versions.is_empty() => format!("{bundle_id} 没有可用版本"),
            None => format!("{bundle_id} 没有稳定版本，使用 --beta 可选择测试版"),
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
    fn version_selection_skips_beta_unless_allowed_or_explicit() {
        let versions = || {
            vec![
                AppVersion {
                    version: "2.0".into(),
                    full_version: "2.0.1".into(),
                    url: "first".into(),
                    beta: true,
                },
                AppVersion {
                    version: "1.0".into(),
                    full_version: "1.0.1".into(),
                    url: "stable".into(),
                    beta: false,
                },
            ]
        };
        assert_eq!(
            select_release(&versions(), None, false).unwrap().url,
            "stable"
        );
        assert_eq!(
            select_release(&versions(), None, true).unwrap().url,
            "first"
        );
        assert_eq!(
            select_release(&versions(), Some("2.0.1"), false)
                .unwrap()
                .url,
            "first"
        );
        assert!(select_release(&versions(), Some("3.0"), true).is_none());
        let beta_only = vec![AppVersion {
            version: "2.0".into(),
            full_version: "2.0.1".into(),
            url: "first".into(),
            beta: true,
        }];
        assert!(select_release(&beta_only, None, false).is_none());
    }

    #[test]
    fn only_strictly_newer_versions_are_updates() {
        assert!(is_newer("1.0", "1.1").unwrap());
        assert!(!is_newer("1.1", "1.0").unwrap());
        assert!(!is_newer("1.0", "1.0").unwrap());
        assert!(is_newer("unknown", "1.0").is_err());
    }
}
