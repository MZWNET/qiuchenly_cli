use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::super::model::DownloadedPackage;
use super::super::pimx;

fn extract_zip(archive: &Path) -> Result<tempfile::TempDir> {
    let staging = tempfile::tempdir().context("创建包解压目录")?;
    let file = fs::File::open(archive).with_context(|| format!("打开 {}", archive.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).with_context(|| format!("读取 ZIP {}", archive.display()))?;
    zip.extract(staging.path())
        .with_context(|| format!("解压 {}", archive.display()))?;
    Ok(staging)
}

pub(super) fn install_zip_package(
    downloaded: &DownloadedPackage,
    context: &mut pimx::InstallContext,
    application_xml: &mut Option<PathBuf>,
) -> Result<()> {
    let package = &downloaded.package;
    let staging =
        extract_zip(&downloaded.path).with_context(|| format!("解压 {}", package.full_name))?;
    context.staging_folder = staging.path().to_path_buf();
    install_pimx_package(downloaded, staging.path(), context, application_xml)
        .with_context(|| format!("安装包 {}", package.full_name))
}

fn install_pimx_package(
    downloaded: &DownloadedPackage,
    staging: &Path,
    context: &pimx::InstallContext,
    application_xml: &mut Option<PathBuf>,
) -> Result<()> {
    let package = &downloaded.package;
    let pimx_path = find_pimx(staging, &package.name)
        .with_context(|| format!("在包 {} 中查找 PIMX", package.full_name))?;
    let bytes = fs::read(&pimx_path).with_context(|| format!("读取 {}", pimx_path.display()))?;
    let xml = pimx::decode_pimx(&bytes)?;
    pimx::install_pimx(&xml, context, application_xml)
        .with_context(|| format!("安装 PIMX {}", package.full_name))
}

fn find_pimx(root: &Path, package_name: &str) -> Result<PathBuf> {
    let expected = format!("{package_name}.pimx");
    let mut candidates = Vec::new();
    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = entry.with_context(|| format!("遍历 {}", root.display()))?;
        if entry.file_type().is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&expected))
        {
            candidates.push(entry.into_path());
        }
    }
    match candidates.len() {
        0 => bail!("未找到 {expected}"),
        1 => Ok(candidates.remove(0)),
        _ => bail!("找到多个 {expected}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_pimx_in_extraction() {
        let staging = tempfile::tempdir().unwrap();
        let package_dir = staging.path().join("AdobePhotoshop28-Core");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("AdobePhotoshop28-Core.pimx"), b"x").unwrap();
        let found = find_pimx(staging.path(), "AdobePhotoshop28-Core").unwrap();
        assert_eq!(found, package_dir.join("AdobePhotoshop28-Core.pimx"));
        assert!(find_pimx(staging.path(), "Missing").is_err());
    }

    #[test]
    fn extracts_zip_with_traversal_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pkg.zip");
        let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("payload/payload.txt", options).unwrap();
        std::io::Write::write_all(&mut writer, b"data").unwrap();
        writer.finish().unwrap();
        let staging = extract_zip(&archive).unwrap();
        assert_eq!(
            fs::read_to_string(staging.path().join("payload/payload.txt")).unwrap(),
            "data"
        );

        let evil = dir.path().join("evil.zip");
        fs::write(&evil, stored_zip(b"../evil.txt", b"evil")).unwrap();
        assert!(extract_zip(&evil).is_err());
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    // 手工构造单文件 stored ZIP，绕过 zip 生成器对路径的校验
    fn stored_zip(name: &[u8], content: &[u8]) -> Vec<u8> {
        let crc = crc32(content).to_le_bytes();
        let size = (content.len() as u32).to_le_bytes();
        let name_len = (name.len() as u16).to_le_bytes();
        let mut out = Vec::new();
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc);
        out.extend_from_slice(&size);
        out.extend_from_slice(&size);
        out.extend_from_slice(&name_len);
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name);
        out.extend_from_slice(content);
        let cd_offset = (out.len() as u32).to_le_bytes();
        out.extend_from_slice(b"PK\x01\x02");
        out.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc);
        out.extend_from_slice(&size);
        out.extend_from_slice(&size);
        out.extend_from_slice(&name_len);
        // extralen/commentlen/disk/internal/external attrs + 本地头偏移
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(name);
        let cd_size = (out.len() as u32 - u32::from_le_bytes(cd_offset)).to_le_bytes();
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&[1, 0, 1, 0]);
        out.extend_from_slice(&cd_size);
        out.extend_from_slice(&cd_offset);
        out.extend_from_slice(&[0, 0]);
        out
    }
}
