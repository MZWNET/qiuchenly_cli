use super::catalog::{resolve_bundle_id, select_release, AppVersion, Release};
use super::client::Client;
use anyhow::{Context, Result};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PreCrackedBind {
    pub version: String,
    #[serde(rename = "fullVersion", default)]
    pub full_version: String,
    #[serde(default)]
    pub beta: bool,
    #[serde(default)]
    pub packages: Vec<PreCrackedPackage>,
}

#[derive(Debug, Deserialize)]
pub struct PreCrackedPackage {
    #[serde(rename = "fileId")]
    pub file_id: String,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct PreCrackedDownload {
    #[serde(rename = "downloadUrl")]
    pub download_url: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub kind: String,
    #[serde(rename = "fileName", default)]
    pub file_name: Option<String>,
}

// 与桌面 encodeURIComponent 的 path segment 编码一致，保留 unreserved 字符
const ENCODE_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn encode_segment(value: &str) -> String {
    utf8_percent_encode(value, ENCODE_SEGMENT).to_string()
}

impl Release for PreCrackedBind {
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

impl Client {
    pub async fn precracked_binds(&self, bundle_id: &str) -> Result<Vec<PreCrackedBind>> {
        self.get(
            &format!(
                "/uploadManage/getBindByBundleId/{}",
                encode_segment(bundle_id)
            ),
            &[],
        )
        .await
    }

    pub async fn precracked_download(
        &self,
        bundle_id: &str,
        bind: &PreCrackedBind,
        package: &PreCrackedPackage,
    ) -> Result<PreCrackedDownload> {
        self.post(
            &format!(
                "/uploadManage/downloadPreCrackedPackage/{}/{}",
                encode_segment(bundle_id),
                encode_segment(&bind.version)
            ),
            &serde_json::json!({
                "fullVersion": bind.full_version,
                "beta": bind.beta,
                "fileId": package.file_id,
            }),
        )
        .await
    }
}

fn local_architecture() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    }
}

fn select_package<'a>(
    packages: &'a [PreCrackedPackage],
    arch: &str,
) -> Option<&'a PreCrackedPackage> {
    packages
        .iter()
        .filter(|package| package.kind != "unavailable")
        .find(|package| package.architecture.as_deref() == Some(arch))
        .or_else(|| {
            packages
                .iter()
                .filter(|package| package.kind != "unavailable")
                .find(|package| package.architecture.as_deref() == Some("universal"))
        })
        .or_else(|| {
            let mut usable = packages
                .iter()
                .filter(|package| package.kind != "unavailable");
            let first = usable.next()?;
            usable.next().is_none().then_some(first)
        })
}

fn decode_file_name(name: Option<&str>) -> Option<String> {
    name.map(|name| match percent_decode_str(name).decode_utf8() {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => name.to_string(),
    })
}

// 服务端 bind.beta 可能错标（如 27.1.0-58756 实为 beta），默认稳定选择需以官方版本表交叉校准
fn effective_beta(bind: &PreCrackedBind, official: &[AppVersion]) -> bool {
    if bind.beta {
        return true;
    }
    if !bind.full_version.is_empty() {
        if let Some(matched) = official
            .iter()
            .find(|version| version.full_version == bind.full_version)
        {
            return matched.beta;
        }
    }
    !official
        .iter()
        .any(|version| !version.beta && version.version == bind.version)
}

pub async fn resolve_download(
    client: &Client,
    app: &str,
    version: Option<&str>,
    beta: bool,
) -> Result<(String, PreCrackedDownload)> {
    let bundle_id = resolve_bundle_id(client, app).await?;
    let binds = client.precracked_binds(&bundle_id).await?;
    let bind = if version.is_none() && !beta {
        let official = client.app_versions(&bundle_id).await?;
        binds
            .iter()
            .find(|bind| !effective_beta(bind, &official))
            .with_context(|| {
                if binds.is_empty() {
                    format!("{bundle_id} 没有预破解版本")
                } else {
                    format!("{bundle_id} 没有稳定的预破解版本，使用 --beta 可选择测试版")
                }
            })?
    } else {
        select_release(&binds, version, beta).with_context(|| match version {
            Some(version) => format!("{bundle_id} 没有预破解版本 {version}"),
            None => format!("{bundle_id} 没有预破解版本"),
        })?
    };
    let package = select_package(&bind.packages, local_architecture())
        .context("该预破解版本没有适合当前架构的安装包")?;
    let download = client
        .precracked_download(&bundle_id, bind, package)
        .await?;
    if download.file_id != package.file_id || download.kind != "package" {
        anyhow::bail!("预破解下载响应与所选安装包不一致");
    }

    Ok((
        bundle_id,
        PreCrackedDownload {
            file_name: decode_file_name(download.file_name.as_deref().or(package.name.as_deref())),
            ..download
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(file_id: &str, architecture: Option<&str>, kind: &str) -> PreCrackedPackage {
        PreCrackedPackage {
            file_id: file_id.into(),
            architecture: architecture.map(str::to_string),
            name: None,
            kind: kind.into(),
        }
    }

    #[test]
    fn package_selection_prefers_native_architecture_then_universal() {
        let packages = vec![
            package("x86", Some("x86_64"), "installer"),
            package("uni", Some("universal"), "installer"),
            package("arm", Some("arm64"), "installer"),
        ];
        assert_eq!(select_package(&packages, "arm64").unwrap().file_id, "arm");
        assert_eq!(select_package(&packages, "x86_64").unwrap().file_id, "x86");

        let universal_only = vec![package("uni", Some("universal"), "installer")];
        assert_eq!(
            select_package(&universal_only, "arm64").unwrap().file_id,
            "uni"
        );

        let single = vec![package("only", Some("x86_64"), "installer")];
        assert_eq!(select_package(&single, "arm64").unwrap().file_id, "only");

        let mixed = vec![
            package("x86", Some("x86_64"), "installer"),
            package("x86-new", Some("x86_64"), "installer"),
        ];
        assert!(select_package(&mixed, "arm64").is_none());
    }

    #[test]
    fn unavailable_packages_are_never_selected() {
        let packages = vec![
            package("bad", Some("arm64"), "unavailable"),
            package("installer", Some("arm64"), "installer"),
        ];
        assert_eq!(
            select_package(&packages, "arm64").unwrap().file_id,
            "installer"
        );
        assert!(select_package(&[package("bad", Some("arm64"), "unavailable")], "arm64").is_none());
    }

    #[test]
    fn download_response_fields_map_and_file_name_decodes() {
        let download: PreCrackedDownload = serde_json::from_str(
            r#"{"downloadUrl":"https://x/dmg","fileId":"f1","kind":"package","fileName":"Parallels%20Desktop%2027.1.0%20%E7%9B%B4%E8%A3%85%E7%A0%B4%E8%A7%A3%E7%89%88.dmg"}"#,
        )
        .unwrap();
        assert_eq!(download.download_url, "https://x/dmg");
        assert_eq!(download.file_id, "f1");
        assert_eq!(download.kind, "package");
        assert_eq!(
            decode_file_name(download.file_name.as_deref()).as_deref(),
            Some("Parallels Desktop 27.1.0 直装破解版.dmg")
        );

        let plain: PreCrackedDownload = serde_json::from_str(
            r#"{"downloadUrl":"https://x/dmg","fileId":"f1","kind":"package"}"#,
        )
        .unwrap();
        assert!(plain.file_name.is_none());
        assert!(decode_file_name(None).is_none());
    }

    #[test]
    fn beta_binds_follow_shared_release_selection() {
        let bind = |version: &str, beta: bool| PreCrackedBind {
            version: version.into(),
            full_version: format!("{version}.0"),
            beta,
            packages: vec![],
        };
        let binds = vec![bind("2.0", true), bind("1.0", false)];
        assert_eq!(select_release(&binds, None, false).unwrap().version, "1.0");
        assert_eq!(select_release(&binds, None, true).unwrap().version, "2.0");
        assert_eq!(
            select_release(&binds, Some("2.0"), false).unwrap().version,
            "2.0"
        );
        assert!(select_release(&binds, Some("3.0"), true).is_none());
    }

    #[test]
    fn mislabeled_binds_are_cross_checked_against_official_versions() {
        let official = |version: &str, full_version: &str, beta: bool| AppVersion {
            version: version.into(),
            full_version: full_version.into(),
            url: "u".into(),
            beta,
        };
        let bind = |version: &str, full_version: &str, beta: bool| PreCrackedBind {
            version: version.into(),
            full_version: full_version.into(),
            beta,
            packages: vec![],
        };
        // 27.1.0 官方条目全为 beta：bind 错标 false 仍按 beta 排除
        let all_beta = vec![official("27.1.0", "58756", true)];
        assert!(effective_beta(&bind("27.1.0", "58756", false), &all_beta));
        // 精确 fullVersion 匹配到官方稳定版 → 稳定
        let stable = vec![official("26.4.1", "56000", false)];
        assert!(!effective_beta(&bind("26.4.1", "56000", false), &stable));
        // 无精确匹配但同 version 有官方稳定版 → 稳定
        assert!(!effective_beta(&bind("26.4.1", "", false), &stable));
        // 官方无任何匹配 → 保守视为 beta
        assert!(effective_beta(&bind("9.9.9", "99999", false), &stable));
        // bind 自身标记 beta 优先
        assert!(effective_beta(&bind("26.4.1", "56000", true), &stable));
    }
}
