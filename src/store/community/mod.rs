mod models;

pub use models::{UserAppDownload, UserAppPackage, UserAppRelease};

use super::catalog::{select_release, Release};
use super::client::Client;
use anyhow::{Context, Result};

impl Client {
    pub async fn user_app_publishers(&self, bundle_id: &str) -> Result<Vec<UserAppRelease>> {
        self.get(
            "/user-apps/publishers",
            &[("bundleId", bundle_id.to_string())],
        )
        .await
    }

    pub async fn user_app_download(
        &self,
        release_id: &str,
        file_id: &str,
    ) -> Result<UserAppDownload> {
        self.post(
            &format!("/user-apps/{release_id}/download-url"),
            &serde_json::json!({ "fileId": file_id }),
        )
        .await
    }
}

impl Release for UserAppRelease {
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

fn select_package<'a>(
    packages: &'a [UserAppPackage],
    kind: &str,
    arch: Option<&str>,
) -> Option<&'a UserAppPackage> {
    match arch {
        Some(arch) => packages
            .iter()
            .find(|package| package.kind == kind && package.architecture.as_deref() == Some(arch))
            .or_else(|| {
                packages.iter().find(|package| {
                    package.kind == kind && package.architecture.as_deref() == Some("universal")
                })
            }),
        None => packages.iter().find(|package| package.kind == kind),
    }
}

pub async fn resolve_download(
    client: &Client,
    bundle_id: &str,
    release: Option<&str>,
    arch: Option<&str>,
    attachment: bool,
    beta: bool,
) -> Result<UserAppDownload> {
    let releases = client.user_app_publishers(bundle_id).await?;
    let selected = match release {
        Some(id) => releases
            .iter()
            .find(|release| release.id == id)
            .with_context(|| format!("找不到发布 {id}"))?,
        None => select_release(&releases, None, beta).with_context(|| {
            if releases.is_empty() {
                format!("{bundle_id} 没有社区发布版本")
            } else {
                format!("{bundle_id} 没有稳定的社区发布版本，使用 --beta 可选择测试版")
            }
        })?,
    };
    let kind = if attachment { "attachment" } else { "package" };
    let package = select_package(&selected.packages, kind, arch)
        .with_context(|| format!("该发布没有可用的 {kind}"))?;
    let mut download = client
        .user_app_download(&selected.id, &package.file_id)
        .await?;
    if download.file_name.is_none() {
        download.file_name = Some(package.file_id.clone());
    }

    Ok(download)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_architecture_never_selects_an_unrelated_package() {
        let mut packages = vec![UserAppPackage {
            file_id: "intel".into(),
            kind: "package".into(),
            architecture: Some("x86_64".into()),
        }];
        assert!(select_package(&packages, "package", Some("arm64")).is_none());
        packages.push(UserAppPackage {
            file_id: "universal".into(),
            kind: "package".into(),
            architecture: Some("universal".into()),
        });
        assert_eq!(
            select_package(&packages, "package", Some("arm64"))
                .unwrap()
                .file_id,
            "universal"
        );
        packages.push(UserAppPackage {
            file_id: "arm".into(),
            kind: "package".into(),
            architecture: Some("arm64".into()),
        });
        assert_eq!(
            select_package(&packages, "package", Some("arm64"))
                .unwrap()
                .file_id,
            "arm"
        );
        assert_eq!(
            select_package(&packages, "package", None).unwrap().file_id,
            "intel"
        );
    }

    #[test]
    fn beta_releases_are_skipped_unless_allowed() {
        let release = |id: &str, beta: bool| UserAppRelease {
            id: id.into(),
            version: id.into(),
            full_version: format!("{id}.0"),
            beta,
            packages: vec![],
        };
        let releases = vec![release("b", true), release("a", false)];
        assert_eq!(select_release(&releases, None, false).unwrap().id, "a");
        assert_eq!(select_release(&releases, None, true).unwrap().id, "b");
        assert_eq!(select_release(&releases, Some("b"), false).unwrap().id, "b");
        let beta_only = vec![release("b", true)];
        assert!(select_release(&beta_only, None, false).is_none());
    }
}
