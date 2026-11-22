use crate::config::{self, DEFAULT_BASE, DEFAULT_CHANNEL};
use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

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
    base: String,
}

impl Client {
    fn with_base(token: Option<&str>, base: &str) -> Result<Self> {
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

        Ok(Self {
            http,
            base: base.to_string(),
        })
    }

    pub fn new(token: Option<String>) -> Result<Self> {
        Self::with_base(token.as_deref(), DEFAULT_BASE)
    }

    pub fn from_channel(name: &str) -> Result<Self> {
        let cfg = config::load()?;

        Self::with_base(cfg.token.as_deref(), cfg.channel_base(name)?)
    }

    pub fn authenticated() -> Result<Self> {
        Self::authenticated_channel(DEFAULT_CHANNEL)
    }

    pub fn authenticated_channel(name: &str) -> Result<Self> {
        let cfg = config::load()?;
        let token = cfg
            .token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("未登录，请先运行 qiuchenly login"))?;

        Self::with_base(Some(token), cfg.channel_base(name)?)
    }

    pub(super) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let resp: Envelope<T> = self
            .http
            .get(format!("{}{path}", self.base))
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
            .post(format!("{}{path}", self.base))
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
