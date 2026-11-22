use anyhow::{bail, Result};
use reqwest::Url;

use crate::config::{self, DEFAULT_BASE, DEFAULT_CHANNEL};

fn normalize_base_url(raw: &str) -> Result<String> {
    let url = Url::parse(raw)?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => bail!("渠道地址仅支持 http 或 https：{scheme}"),
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("渠道地址不能包含查询参数或片段");
    }

    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn list() -> Result<()> {
    let cfg = config::load()?;
    println!("{DEFAULT_CHANNEL:<12} {DEFAULT_BASE}");
    for (name, base) in &cfg.channels {
        println!("{name:<12} {base}");
    }

    Ok(())
}

pub fn add(name: &str, base_url: &str) -> Result<()> {
    if name == DEFAULT_CHANNEL {
        bail!("{DEFAULT_CHANNEL} 为内置渠道，不能添加");
    }
    let mut cfg = config::load()?;
    if cfg.channels.contains_key(name) {
        bail!("渠道 {name} 已存在");
    }
    let base = normalize_base_url(base_url)?;
    cfg.channels.insert(name.to_string(), base);
    config::save(&cfg)?;
    println!("已添加渠道 {name}");

    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    if name == DEFAULT_CHANNEL {
        bail!("{DEFAULT_CHANNEL} 为内置渠道，不能移除");
    }
    let mut cfg = config::load()?;
    if cfg.channels.remove(name).is_none() {
        bail!("未知渠道：{name}");
    }
    config::save(&cfg)?;
    println!("已移除渠道 {name}");

    Ok(())
}
