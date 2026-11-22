use super::application::parse_application;
use super::model::{Application, Product};

mod model;
mod resolve;

use self::model::CatalogResponse;
use self::resolve::resolve_response;
use anyhow::{bail, Context, Result};
use tokio::process::Command;

const CATALOG_URL: &str =
    "https://prod-rel-ffc-ccm.oobesaas.adobe.com/adobe-ffc-external/core/v6/products/all";
const APPLICATION_URL: &str = "https://cdn-ffc.oobesaas.adobe.com/core/v3/applications";
const API_KEY: &str = "qiolito74e5uKpdpfjtJyu7bXXWEJdO2d";
const APP_ID: &str = "accc-apps-panel-desktop";
const USER_AGENT: &str = "Adobe Application Manager 2.0";
const PLATFORMS: &str = "macarm64,macuniversal,osx10-64,osx10";

pub(super) struct CatalogClient;

impl CatalogClient {
    pub(super) fn new() -> Result<Self> {
        Ok(Self)
    }

    pub(super) fn download_headers(&self) -> Vec<(String, String)> {
        vec![
            ("X-Adobe-App-Id".to_owned(), APP_ID.to_owned()),
            ("X-Api-Key".to_owned(), API_KEY.to_owned()),
            ("User-Agent".to_owned(), "Creative Cloud".to_owned()),
        ]
    }

    async fn get_body(
        &self,
        url: &str,
        query: Option<&[(&str, &str)]>,
        build_guid: Option<&str>,
    ) -> Result<String> {
        let mut parsed =
            reqwest::Url::parse(url).with_context(|| format!("无效的 Adobe 请求地址: {url}"))?;
        if let Some(query) = query {
            let mut pairs = parsed.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let mut command = Command::new("curl");
        command
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "--compressed",
                "--location",
                "--max-time",
                "300",
            ])
            .arg("--header")
            .arg(format!("X-Adobe-App-Id: {APP_ID}"))
            .arg("--header")
            .arg(format!("X-Api-Key: {API_KEY}"))
            .arg("--header")
            .arg(format!("User-Agent: {USER_AGENT}"));
        if let Some(build_guid) = build_guid {
            command
                .arg("--header")
                .arg(format!("x-adobe-build-guid: {build_guid}"));
        }
        let output = command
            .arg("--")
            .arg(parsed.as_str())
            .output()
            .await
            .context("运行 curl 失败")?;
        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            bail!("Adobe HTTP 请求失败: {url} {error}");
        }
        let body = String::from_utf8(output.stdout)
            .with_context(|| format!("Adobe 响应不是有效的 UTF-8: {url}"))?;
        if body.trim().is_empty() {
            bail!("Adobe 响应为空: {url}");
        }
        Ok(body)
    }

    pub(super) async fn resolve_products(
        &self,
        app: &str,
        version: Option<&str>,
        beta: bool,
    ) -> Result<Option<Vec<Product>>> {
        let query: &[(&str, &str)] = &[
            ("channel", "ccm"),
            ("channel", "sti"),
            ("platform", PLATFORMS),
            ("_type", "json"),
            ("productType", "Desktop"),
            ("payload", "true"),
            ("filterTranslatedApps", "true"),
            ("hidpi", "true"),
        ];
        let sti_query: &[(&str, &str)] = &[
            ("channel", "sti"),
            ("platform", PLATFORMS),
            ("_type", "json"),
            ("productType", "Desktop"),
        ];
        let (body, sti_body) = tokio::join!(
            self.get_body(CATALOG_URL, Some(query), None),
            self.get_body(CATALOG_URL, Some(sti_query), None),
        );
        let mut response: CatalogResponse =
            serde_json::from_str(&body?).context("Adobe 产品目录 JSON 解析失败")?;
        let sti_response: CatalogResponse =
            serde_json::from_str(&sti_body?).context("Adobe STI 产品目录 JSON 解析失败")?;
        response.channels.channel = response
            .channels
            .channel
            .append(sti_response.channels.channel);
        resolve_response(&response, app, version, beta)
    }

    pub(super) async fn fetch_application(
        &self,
        product: &Product,
    ) -> Result<(Application, String)> {
        let query: &[(&str, &str)] = &[
            ("sapCode", product.sap_code.as_str()),
            ("baseVersion", product.base_version.as_str()),
            ("platform", product.platform.as_str()),
        ];
        let raw = self
            .get_body(APPLICATION_URL, Some(query), Some(&product.build_guid))
            .await
            .with_context(|| format!("获取 {} application.json 失败", product.sap_code))?;
        let application = parse_application(&raw, product)
            .with_context(|| format!("解析 {} application.json 失败", product.sap_code))?;
        Ok((application, raw))
    }

    pub(super) async fn get_text(&self, url: &str) -> Result<String> {
        self.get_body(url, None, None).await
    }
}
