use super::client::Client;
use super::models::{UserAppDownload, UserAppPackage};
use anyhow::{Context, Result};

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
) -> Result<UserAppDownload> {
    let releases = client.user_app_publishers(bundle_id).await?;
    let selected = match release {
        Some(id) => releases
            .iter()
            .find(|release| release.id == id)
            .with_context(|| format!("找不到发布 {id}"))?,
        None => releases
            .first()
            .with_context(|| format!("{bundle_id} 没有社区发布版本"))?,
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
}
