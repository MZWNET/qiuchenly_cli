use super::application::parse_application;
use super::model::{platform_preference, Application, Dependency, Product};
use super::response::OneOrMany;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::process::Command;

const CATALOG_URL: &str =
    "https://prod-rel-ffc-ccm.oobesaas.adobe.com/adobe-ffc-external/core/v6/products/all";
const APPLICATION_URL: &str = "https://cdn-ffc.oobesaas.adobe.com/core/v3/applications";
const API_KEY: &str = "qiolito74e5uKpdpfjtJyu7bXXWEJdO2d";
const APP_ID: &str = "accc-apps-panel-desktop";
const USER_AGENT: &str = "Adobe Application Manager 2.0";
const PLATFORMS: &str = "macarm64,macuniversal,osx10-64,osx10";

pub(super) struct CatalogClient;

#[derive(Deserialize)]
struct CatalogResponse {
    #[serde(default)]
    channels: CatalogChannels,
}

#[derive(Default, Deserialize)]
struct CatalogChannels {
    #[serde(default)]
    channel: OneOrMany<CatalogChannel>,
}

#[derive(Deserialize)]
struct CatalogChannel {
    #[serde(default)]
    cdn: CatalogCdn,
    #[serde(default)]
    products: ChannelProducts,
}

#[derive(Default, Deserialize)]
struct CatalogCdn {
    #[serde(default)]
    secure: String,
}

#[derive(Default, Deserialize)]
struct ChannelProducts {
    #[serde(default)]
    product: OneOrMany<CatalogProduct>,
}

#[derive(Deserialize)]
struct CatalogProduct {
    id: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
    version: String,
    #[serde(default)]
    platforms: ProductPlatforms,
}

#[derive(Default, Deserialize)]
struct ProductPlatforms {
    #[serde(default)]
    platform: OneOrMany<ProductPlatform>,
}

#[derive(Deserialize)]
struct ProductPlatform {
    id: String,
    #[serde(default, rename = "languageSet")]
    language_set: OneOrMany<PlatformLanguageSet>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlatformLanguageSet {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_version: Option<String>,
    #[serde(default)]
    build_guid: Option<String>,
    #[serde(default)]
    urls: LanguageSetUrls,
    #[serde(default)]
    dependencies: LanguageSetDependencies,
}

#[derive(Default, Deserialize)]
struct LanguageSetUrls {
    #[serde(default, rename = "manifestURL")]
    manifest_url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageSetDependencies {
    #[serde(default)]
    dependency: OneOrMany<LanguageSetDependency>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanguageSetDependency {
    #[serde(default)]
    sap_code: String,
    #[serde(default)]
    base_version: String,
}

fn flatten_entries(response: &CatalogResponse) -> Vec<Product> {
    let preference = platform_preference();
    let mut entries = Vec::new();
    for channel in response.channels.channel.iter() {
        for product in channel.products.product.iter() {
            let Some(platform) = preference
                .iter()
                .find_map(|wanted| product.platforms.platform.iter().find(|p| p.id == *wanted))
            else {
                continue;
            };
            let language_sets = platform.language_set.as_slice();
            if language_sets.is_empty() {
                continue;
            }
            let set = language_sets
                .iter()
                .find(|set| set.name.as_deref() == Some("LS20"))
                .unwrap_or(&language_sets[0]);
            let dependencies = set
                .dependencies
                .dependency
                .iter()
                .map(|dep| Dependency {
                    sap_code: dep.sap_code.clone(),
                    base_version: dep.base_version.clone(),
                })
                .collect();
            entries.push(Product {
                base_version: set
                    .base_version
                    .clone()
                    .filter(|version| !version.is_empty())
                    .unwrap_or_else(|| product.version.clone()),
                build_guid: set.build_guid.clone().unwrap_or_default(),
                manifest_url: set.urls.manifest_url.clone(),
                dependencies,
                sap_code: product.id.clone(),
                display_name: product.display_name.clone(),
                version: product.version.clone(),
                platform: platform.id.clone(),
                cdn: channel.cdn.secure.clone(),
            });
        }
    }
    entries
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn matches_query(app: &str, beta: bool, sap_code: &str, display_name: &str) -> bool {
    if sap_code.eq_ignore_ascii_case(app) {
        return true;
    }
    let stripped = app.strip_prefix("com.adobe.").unwrap_or(app);
    if sap_code.eq_ignore_ascii_case(stripped) {
        return true;
    }
    if beta && sap_code.eq_ignore_ascii_case(&format!("{stripped}BETA")) {
        return true;
    }
    normalize(stripped) == normalize(display_name)
}

fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    match version_compare::compare(left, right) {
        Ok(version_compare::Cmp::Lt) => std::cmp::Ordering::Less,
        Ok(version_compare::Cmp::Gt) => std::cmp::Ordering::Greater,
        Ok(_) => std::cmp::Ordering::Equal,
        Err(_) => left.cmp(right),
    }
}

#[cfg(test)]
fn resolve_from_catalog(
    catalog: &str,
    app: &str,
    version: Option<&str>,
    beta: bool,
) -> Result<Option<Vec<Product>>> {
    let response: CatalogResponse =
        serde_json::from_str(catalog).context("Adobe 产品目录 JSON 解析失败")?;
    resolve_response(&response, app, version, beta)
}

fn resolve_response(
    response: &CatalogResponse,
    app: &str,
    version: Option<&str>,
    beta: bool,
) -> Result<Option<Vec<Product>>> {
    let entries = flatten_entries(response);
    let matched: Vec<&Product> = entries
        .iter()
        .filter(|entry| matches_query(app, beta, &entry.sap_code, &entry.display_name))
        .collect();
    if matched.is_empty() {
        return Ok(None);
    }
    let pool: Vec<&Product> = matched
        .into_iter()
        .filter(|entry| match version {
            Some(wanted) => entry.version == wanted || entry.base_version == wanted,
            None => beta || !entry.sap_code.ends_with("BETA"),
        })
        .collect();
    let main = pool
        .iter()
        .max_by(|a, b| compare_versions(&a.version, &b.version))
        .copied()
        .ok_or_else(|| match version {
            Some(version) => anyhow!("Adobe 产品 {app} 没有版本 {version}"),
            None => anyhow!("Adobe 产品 {app} 没有可用版本"),
        })?;

    let mut by_sap: HashMap<&str, Vec<&Product>> = HashMap::new();
    for entry in &entries {
        by_sap
            .entry(entry.sap_code.as_str())
            .or_default()
            .push(entry);
    }
    let mut resolved: Vec<Product> = Vec::new();
    let mut requirements: HashMap<String, String> = HashMap::new();
    let mut stack: Vec<String> = vec![main.sap_code.clone()];
    collect_dependencies(main, &by_sap, &mut resolved, &mut requirements, &mut stack)?;
    resolved.push(main.clone());
    for product in &mut resolved {
        if product.display_name.is_empty() {
            product.display_name.clone_from(&product.sap_code);
        }
    }
    Ok(Some(resolved))
}

fn collect_dependencies<'a>(
    entry: &'a Product,
    by_sap: &HashMap<&str, Vec<&'a Product>>,
    resolved: &mut Vec<Product>,
    requirements: &mut HashMap<String, String>,
    stack: &mut Vec<String>,
) -> Result<()> {
    for dependency in &entry.dependencies {
        if dependency.sap_code.is_empty() {
            continue;
        }
        if stack.iter().any(|sap| sap == &dependency.sap_code) {
            bail!("Adobe 依赖循环: {}", stack.join(" -> "));
        }
        if let Some(required) = requirements.get(&dependency.sap_code) {
            if required != &dependency.base_version {
                bail!(
                    "Adobe 依赖 {} 版本冲突: {} 与 {}",
                    dependency.sap_code,
                    required,
                    dependency.base_version
                );
            }
            continue;
        }
        let candidates = by_sap
            .get(dependency.sap_code.as_str())
            .ok_or_else(|| anyhow!("Adobe 依赖 {} 在目录中不存在", dependency.sap_code))?;
        let candidate = candidates
            .iter()
            .copied()
            .filter(|candidate| {
                dependency.base_version.is_empty()
                    || candidate.base_version == dependency.base_version
            })
            .max_by(|a, b| compare_versions(&a.version, &b.version))
            .ok_or_else(|| {
                anyhow!(
                    "Adobe 依赖 {} 没有版本 {}",
                    dependency.sap_code,
                    dependency.base_version
                )
            })?;
        requirements.insert(dependency.sap_code.clone(), dependency.base_version.clone());
        stack.push(dependency.sap_code.clone());
        collect_dependencies(candidate, by_sap, resolved, requirements, stack)?;
        stack.pop();
        resolved.push(candidate.clone());
    }
    Ok(())
}

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
        if product.build_guid.is_empty() {
            if product.manifest_url.is_some() {
                bail!(
                    "产品 {} 使用 manifest 安装，没有 application.json",
                    product.sap_code
                );
            }
            bail!("缺少必需参数: buildGuid 或 manifestURL");
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog_json() -> String {
        let photoshop = |version: &str, guid: &str| {
            json!({
                "type": "Desktop",
                "id": "PHSP",
                "displayName": "Photoshop",
                "version": version,
                "platforms": {"platform": [{
                    "id": "macuniversal",
                    "languageSet": [{
                        "name": "LS20",
                        "baseVersion": version,
                        "buildGuid": guid,
                        "urls": {},
                        "dependencies": {"dependency": [
                            {"sapCode": "CORE", "baseVersion": "1.0"},
                        ]},
                    }],
                }]},
            })
        };
        json!({
            "channels": {"channel": [
                {
                    "name": "sti",
                    "cdn": {"secure": "https://sti.example.com"},
                    "products": {"product": [{
                        "type": "Desktop",
                        "id": "CORE",
                        "displayName": "STI_Color_HD",
                        "version": "1.0.12",
                        "platforms": {"platform": [{
                            "id": "osx10",
                            "languageSet": {
                                "name": "LS20",
                                "baseVersion": "1.0",
                                "buildGuid": "core-guid",
                                "urls": {"lbsURL": "/x/"},
                                "dependencies": {},
                            },
                        }]},
                    }]},
                },
                {
                    "name": "ccm",
                    "cdn": {"secure": "https://ccm.example.com"},
                    "products": {"product": [
                        photoshop("28.0", "phsp-28-guid"),
                        photoshop("27.0", "phsp-27-guid"),
                        json!({
                            "type": "Desktop",
                            "id": "PHSPBETA",
                            "displayName": "Photoshop",
                            "version": "29.0",
                            "platforms": {"platform": [{
                                "id": "macuniversal",
                                "languageSet": [{
                                    "name": "LS20",
                                    "baseVersion": "29.0",
                                    "buildGuid": "phsp-29-beta-guid",
                                    "urls": {},
                                    "dependencies": {"dependency": [
                                        {"sapCode": "CORE", "baseVersion": "1.0"},
                                    ]},
                                }],
                            }]},
                        }),
                        json!({
                            "type": "Desktop",
                            "id": "APRO",
                            "displayName": "Acrobat",
                            "version": "26.1",
                            "platforms": {"platform": [{
                                "id": "macuniversal",
                                "languageSet": {
                                    "name": "WWMUI",
                                    "urls": {"manifestURL":
                                        "/AdobeProducts/APRO/26_1/manifest.xml"},
                                },
                            }]},
                        }),
                    ]},
                },
            ]},
        })
        .to_string()
    }

    fn entry<'a>(products: &'a [Product], sap_code: &str) -> &'a Product {
        products.iter().find(|p| p.sap_code == sap_code).unwrap()
    }

    #[test]
    fn resolves_dependencies_before_main() {
        let products = resolve_from_catalog(&catalog_json(), "com.adobe.Photoshop", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].sap_code, "CORE");
        assert_eq!(products[0].base_version, "1.0");
        let main = entry(&products, "PHSP");
        assert_eq!(main.version, "28.0");
        assert_eq!(main.base_version, "28.0");
        assert_eq!(main.platform, "macuniversal");
        assert_eq!(main.build_guid, "phsp-28-guid");
        assert_eq!(main.cdn, "https://ccm.example.com");
    }

    #[test]
    fn resolves_dependencies_from_sti_catalog() {
        let catalog: serde_json::Value = serde_json::from_str(&catalog_json()).unwrap();
        let channels = catalog["channels"]["channel"].as_array().unwrap();
        let main = json!({"channels": {"channel": channels[1]}}).to_string();
        let sti = json!({"channels": {"channel": channels[0]}}).to_string();
        assert!(resolve_from_catalog(&main, "PHSP", None, false).is_err());

        let mut response: CatalogResponse = serde_json::from_str(&main).unwrap();
        let sti_response: CatalogResponse = serde_json::from_str(&sti).unwrap();
        response.channels.channel = response
            .channels
            .channel
            .append(sti_response.channels.channel);
        let products = resolve_response(&response, "PHSP", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0].sap_code, "CORE");
        assert_eq!(products[0].cdn, "https://sti.example.com");
        assert_eq!(products[1].sap_code, "PHSP");
    }

    #[test]
    fn matches_sap_code_case_insensitively() {
        let products = resolve_from_catalog(&catalog_json(), "phsp", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(entry(&products, "PHSP").build_guid, "phsp-28-guid");
    }

    #[test]
    fn empty_display_names_match_before_falling_back_to_sap_codes() {
        let product = |sap_code: &str, guid: &str, dependencies: serde_json::Value| {
            json!({
                "id": sap_code,
                "version": "1.0",
                "platforms": {"platform": {"id": "macuniversal", "languageSet": {
                    "buildGuid": guid,
                    "dependencies": {"dependency": dependencies}
                }}}
            })
        };
        let catalog = json!({"channels": {"channel": {"products": {"product": [
            product("DEP", "dependency", json!([])),
            product("MAIN", "first", json!([{"sapCode": "DEP", "baseVersion": "1.0"}])),
            product("MAIN", "last", json!([{"sapCode": "DEP", "baseVersion": "1.0"}]))
        ]}}}})
        .to_string();
        assert!(resolve_from_catalog(&catalog, "Other", None, false)
            .unwrap()
            .is_none());
        let products = resolve_from_catalog(&catalog, "!!!", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(products[0].display_name, "DEP");
        assert_eq!(products[1].display_name, "MAIN");
        assert_eq!(products[1].build_guid, "last");
        let products = resolve_from_catalog(&catalog, "MAIN", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(products[1].display_name, "MAIN");
    }

    #[test]
    fn explicit_version_selects_exact_match() {
        let products = resolve_from_catalog(&catalog_json(), "Photoshop", Some("27.0"), false)
            .unwrap()
            .unwrap();
        assert_eq!(entry(&products, "PHSP").build_guid, "phsp-27-guid");
        // 显式版本必须精确匹配，27 不应命中 27.0
        assert!(resolve_from_catalog(&catalog_json(), "Photoshop", Some("27"), false).is_err());
    }

    #[test]
    fn auto_version_skips_beta_but_explicit_version_allows_it() {
        let products = resolve_from_catalog(&catalog_json(), "Photoshop", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(entry(&products, "PHSP").version, "28.0");
        let products = resolve_from_catalog(&catalog_json(), "PHSP", None, true)
            .unwrap()
            .unwrap();
        assert_eq!(products.last().unwrap().sap_code, "PHSPBETA");
        let products = resolve_from_catalog(&catalog_json(), "Photoshop", Some("29.0"), false)
            .unwrap()
            .unwrap();
        assert_eq!(products.last().unwrap().sap_code, "PHSPBETA");
    }

    #[test]
    fn non_adobe_input_returns_none() {
        assert!(resolve_from_catalog(&catalog_json(), "WeChat", None, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn unknown_version_is_an_error() {
        assert!(resolve_from_catalog(&catalog_json(), "Photoshop", Some("99.0"), false).is_err());
    }

    #[test]
    fn apro_entry_uses_manifest_and_falls_back_base_version() {
        let products = resolve_from_catalog(&catalog_json(), "acrobat", None, false)
            .unwrap()
            .unwrap();
        let apro = entry(&products, "APRO");
        assert_eq!(
            apro.manifest_url.as_deref(),
            Some("/AdobeProducts/APRO/26_1/manifest.xml")
        );
        assert_eq!(apro.base_version, "26.1");
        assert_eq!(apro.build_guid, "");
    }

    #[test]
    fn detects_dependency_cycles_and_conflicts() {
        let depends_on = |sap_code: &str| {
            json!({
                "id": sap_code,
                "displayName": sap_code,
                "version": "1.0",
                "platforms": {"platform": [{
                    "id": "macuniversal",
                    "languageSet": {"dependencies": {"dependency": [{
                        "sapCode": if sap_code == "AAA" { "BBB" } else { "AAA" },
                        "baseVersion": "1.0",
                    }]}},
                }]},
            })
        };
        let cycle = json!({
            "channels": {"channel": [{
                "cdn": {"secure": "https://x.example.com"},
                "products": {"product": [depends_on("AAA"), depends_on("BBB")]},
            }]},
        })
        .to_string();
        let error = resolve_from_catalog(&cycle, "AAA", None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("循环"), "{error}");

        let sub = |base_version: &str| {
            json!({
                "id": "SUB",
                "displayName": "Sub",
                "version": format!("{base_version}.1"),
                "platforms": {"platform": [{
                    "id": "macuniversal",
                    "languageSet": {"baseVersion": base_version},
                }]},
            })
        };
        let conflict = json!({
            "channels": {"channel": [{
                "cdn": {"secure": "https://x.example.com"},
                "products": {"product": [
                    json!({
                        "id": "MAIN",
                        "displayName": "Main",
                        "version": "1.0",
                        "platforms": {"platform": [{
                            "id": "macuniversal",
                            "languageSet": {"dependencies": {"dependency": [
                                {"sapCode": "SUB", "baseVersion": "1.0"},
                                {"sapCode": "SUB", "baseVersion": "2.0"},
                            ]}},
                        }]},
                    }),
                    sub("1.0"),
                    sub("2.0"),
                ]},
            }]},
        })
        .to_string();
        let error = resolve_from_catalog(&conflict, "Main", None, false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("冲突"), "{error}");
    }

    #[test]
    fn dependency_resolves_exact_base_version() {
        let dep = |version: &str, base_version: &str, guid: &str| {
            json!({
                "id": "DEP",
                "displayName": "Dep",
                "version": version,
                "platforms": {"platform": [{
                    "id": "macuniversal",
                    "languageSet": {"baseVersion": base_version, "buildGuid": guid},
                }]},
            })
        };
        let catalog = json!({
            "channels": {"channel": [{
                "cdn": {"secure": "https://x.example.com"},
                "products": {"product": [
                    json!({
                        "id": "MAIN",
                        "displayName": "Main",
                        "version": "3.0",
                        "platforms": {"platform": [{
                            "id": "macuniversal",
                            "languageSet": {"dependencies": {"dependency": [
                                {"sapCode": "DEP", "baseVersion": "1.0"},
                            ]}},
                        }]},
                    }),
                    dep("9.9", "9.0", "dep-9"),
                    dep("1.5", "1.0", "dep-1"),
                ]},
            }]},
        })
        .to_string();
        let products = resolve_from_catalog(&catalog, "Main", None, false)
            .unwrap()
            .unwrap();
        assert_eq!(products[0].sap_code, "DEP");
        assert_eq!(products[0].build_guid, "dep-1");
    }
}
