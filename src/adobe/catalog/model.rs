use serde::Deserialize;

use super::super::response::OneOrMany;

#[derive(Deserialize)]
pub(super) struct CatalogResponse {
    #[serde(default)]
    pub(super) channels: CatalogChannels,
}

#[derive(Default, Deserialize)]
pub(super) struct CatalogChannels {
    #[serde(default)]
    pub(super) channel: OneOrMany<CatalogChannel>,
}

#[derive(Deserialize)]
pub(super) struct CatalogChannel {
    #[serde(default)]
    pub(super) cdn: CatalogCdn,
    #[serde(default)]
    pub(super) products: ChannelProducts,
}

#[derive(Default, Deserialize)]
pub(super) struct CatalogCdn {
    #[serde(default)]
    pub(super) secure: String,
}

#[derive(Default, Deserialize)]
pub(super) struct ChannelProducts {
    #[serde(default)]
    pub(super) product: OneOrMany<CatalogProduct>,
}

#[derive(Deserialize)]
pub(super) struct CatalogProduct {
    pub(super) id: String,
    #[serde(default, rename = "displayName")]
    pub(super) display_name: String,
    pub(super) version: String,
    #[serde(default)]
    pub(super) platforms: ProductPlatforms,
}

#[derive(Default, Deserialize)]
pub(super) struct ProductPlatforms {
    #[serde(default)]
    pub(super) platform: OneOrMany<ProductPlatform>,
}

#[derive(Deserialize)]
pub(super) struct ProductPlatform {
    pub(super) id: String,
    #[serde(default, rename = "languageSet")]
    pub(super) language_set: OneOrMany<PlatformLanguageSet>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlatformLanguageSet {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) base_version: Option<String>,
    #[serde(default)]
    pub(super) build_guid: Option<String>,
    #[serde(default)]
    pub(super) urls: LanguageSetUrls,
    #[serde(default)]
    pub(super) dependencies: LanguageSetDependencies,
}

#[derive(Default, Deserialize)]
pub(super) struct LanguageSetUrls {
    #[serde(default, rename = "manifestURL")]
    pub(super) manifest_url: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LanguageSetDependencies {
    #[serde(default)]
    pub(super) dependency: OneOrMany<LanguageSetDependency>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LanguageSetDependency {
    #[serde(default)]
    pub(super) sap_code: String,
    #[serde(default)]
    pub(super) base_version: String,
}
