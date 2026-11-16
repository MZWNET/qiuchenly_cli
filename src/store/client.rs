use super::models::{
    AppVersion, CatalogList, Category, CategoryApps, GuiUpdate, LoginResponse, Popular,
    PopularBind, PublicKey, User, UserAppDownload, UserAppRelease,
};
use crate::config;
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const DEFAULT_BASE: &str = "https://api1.xkji.com/api/v1";

#[derive(Deserialize)]
struct Envelope<T> {
    success: bool,
    message: Option<String>,
    data: Option<T>,
}

impl<T> Envelope<T> {
    fn into_result(self) -> Result<T> {
        if self.success {
            self.data.ok_or_else(|| anyhow::anyhow!("响应缺少 data"))
        } else {
            Err(anyhow::anyhow!(self
                .message
                .unwrap_or_else(|| "请求失败".into())))
        }
    }
}

pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {token}").parse()?,
            );
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("qiuchenly-cli/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(120))
            .build()?;

        Ok(Self { http })
    }

    pub fn from_config() -> Result<Self> {
        Self::new(config::load()?.token)
    }

    pub fn authenticated() -> Result<Self> {
        let token = config::load()?
            .token
            .ok_or_else(|| anyhow::anyhow!("未登录，请先运行 qiuchenly login"))?;

        Self::new(Some(token))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let resp: Envelope<T> = self
            .http
            .get(format!("{DEFAULT_BASE}{path}"))
            .query(query)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        resp.into_result()
    }

    pub async fn get_json(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value> {
        self.get(path, query).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let raw = self
            .http
            .post(format!("{DEFAULT_BASE}{path}"))
            .json(body)
            .send()
            .await?;
        if raw.status() == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("用户名或密码错误（或 token 已失效）");
        }
        let resp: Envelope<T> = raw.error_for_status()?.json().await?;

        resp.into_result()
    }

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

    pub async fn public_key(&self) -> Result<PublicKey> {
        self.get("/auth/public-key", &[]).await
    }

    pub async fn login(&self, username: &str, encrypted_password: &str) -> Result<LoginResponse> {
        self.post(
            "/auth/login",
            &serde_json::json!({ "username": username, "password": encrypted_password }),
        )
        .await
    }

    pub async fn user(&self) -> Result<User> {
        self.get("/auth/user", &[]).await
    }

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
