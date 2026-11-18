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

    pub(super) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
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

    pub(super) async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
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
}
