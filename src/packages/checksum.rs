use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub(crate) fn verify_sha256(path: &Path, expected: &str, label: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut file = File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut actual = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        write!(actual, "{byte:02x}").expect("写入十六进制字符不会失败");
    }
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("{label} SHA256 校验失败: 期望 {expected}，实际 {actual}");
    }

    Ok(())
}
