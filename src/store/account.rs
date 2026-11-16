use super::client::Client;
use crate::config;
use anyhow::{Context, Result};
use base64::Engine;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Oaep, RsaPublicKey};

pub async fn login(username: &str, password: &str) -> Result<()> {
    let client = Client::new(None)?;
    let public_key = client.public_key().await?;
    let key =
        RsaPublicKey::from_public_key_pem(&public_key.public_key).context("解析登录公钥失败")?;
    let encrypted = key
        .encrypt(
            &mut rand::thread_rng(),
            Oaep::new::<sha2::Sha256>(),
            password.as_bytes(),
        )
        .context("密码加密失败")?;
    let password = base64::engine::general_purpose::STANDARD.encode(encrypted);
    let response = client.login(username, &password).await?;
    let mut cfg = config::load()?;
    cfg.token = Some(response.token);
    cfg.refresh_token = response.refresh_token;
    cfg.username = Some(username.to_string());

    config::save(&cfg)
}

pub async fn status() -> Result<Option<String>> {
    let cfg = config::load()?;
    match cfg.token {
        Some(token) => Ok(Some(Client::new(Some(token))?.user().await?.username)),
        None => Ok(None),
    }
}

pub fn logout() -> Result<()> {
    let mut cfg = config::load()?;
    cfg.token = None;
    cfg.refresh_token = None;
    cfg.username = None;

    config::save(&cfg)
}
