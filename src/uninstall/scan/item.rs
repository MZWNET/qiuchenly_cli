use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

#[derive(Clone)]
pub(in crate::uninstall) struct ScanItem {
    pub path: PathBuf,
    pub deletable: bool,
    pub confidence: &'static str,
    pub size_bytes: Option<u64>,
    pub reason: Option<String>,
    pub shared_with: Vec<String>,
    pub identity: Identity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::uninstall) struct Identity {
    dev: u64,
    ino: u64,
    file_type: fs::FileType,
}

impl Identity {
    pub fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            file_type: metadata.file_type(),
        }
    }
}

impl ScanItem {
    pub fn app(path: PathBuf) -> std::io::Result<Self> {
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            deletable: true,
            confidence: "high",
            size_bytes: None,
            reason: None,
            shared_with: Vec::new(),
            identity: Identity::from_metadata(&metadata),
        })
    }
}
