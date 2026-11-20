use anyhow::{bail, Context, Result};
use percent_encoding::percent_decode_str;
use reqwest::Url;
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;

pub struct Downloader {
    program: PathBuf,
}

fn download_name(url: &Url, file_name: Option<&str>) -> Result<String> {
    let name = match file_name {
        Some(name) => name.to_owned(),
        None => {
            let encoded = url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or_default();
            percent_decode_str(encoded)
                .decode_utf8()
                .context("下载文件名不是有效的 UTF-8")?
                .into_owned()
        }
    };
    if !matches!(Path::new(&name).components().next(), Some(Component::Normal(part)) if part == name.as_str())
        || Path::new(&name).components().count() != 1
    {
        bail!("无效的下载文件名: {name}");
    }

    Ok(name)
}

impl Downloader {
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }

    pub async fn download(
        &self,
        url: &str,
        dest_dir: &Path,
        file_name: Option<&str>,
    ) -> Result<PathBuf> {
        self.download_with_headers(url, dest_dir, file_name, &[])
            .await
    }

    pub async fn download_with_headers(
        &self,
        url: &str,
        dest_dir: &Path,
        file_name: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<PathBuf> {
        let parsed = Url::parse(url).with_context(|| format!("无效的下载地址: {url}"))?;
        let name = download_name(&parsed, file_name)?;
        for (header, value) in headers {
            if header.is_empty()
                || header.bytes().any(|b| b <= b' ')
                || value.bytes().any(|b| b == b'\n' || b == b'\r')
            {
                bail!("无效的下载 header: {header}");
            }
        }
        let mut command = Command::new(&self.program);
        command.args([
            "--continue=true",
            "--file-allocation=none",
            "--console-log-level=warn",
            "--summary-interval=0",
            "--max-connection-per-server=8",
            "--split=8",
            "--min-split-size=1M",
            "--auto-file-renaming=false",
        ]);
        for (header, value) in headers {
            command.arg(format!("--header={header}: {value}"));
        }
        let status = command
            .arg("--dir")
            .arg(dest_dir)
            .arg("--out")
            .arg(&name)
            .arg("--")
            .arg(url)
            .status()
            .await
            .with_context(|| format!("运行 {}", self.program.display()))?;
        if !status.success() {
            bail!("aria2c 退出码 {:?}", status.code());
        }

        Ok(dest_dir.join(name))
    }
}

#[cfg(test)]
mod tests {
    use super::download_name;
    use reqwest::Url;

    #[test]
    fn decodes_url_filename_and_rejects_traversal() {
        let url = Url::parse("https://example.com/My%20App.zip?token=abc").unwrap();
        assert_eq!(download_name(&url, None).unwrap(), "My App.zip");
        for url in [
            "https://example.com/%2e%2e%2fapp.zip",
            "https://example.com/%2Ftmp%2Fapp.zip",
        ] {
            assert!(download_name(&Url::parse(url).unwrap(), None).is_err());
        }
        assert!(download_name(&url, Some("../app.zip")).is_err());
    }
}
