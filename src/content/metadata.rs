use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug)]
pub(super) struct DirectoryMetadata {
    pub subdirs: BTreeMap<String, DirectoryMetadata>,
    pub pages: Vec<PageMetadata>,
    pub assets: Vec<AssetMetadata>,
}

#[derive(Debug)]
pub(super) enum FileMetadata {
    Page(PageMetadata),
    Asset(AssetMetadata),
}

#[derive(Debug, Serialize)]
pub(super) struct PageMetadata {
    pub draft: bool,
    pub slug: String,
    pub path: Utf8PathBuf,
    pub title: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub(super) struct AssetMetadata {
    pub path: Utf8PathBuf,
}

impl AssetMetadata {
    pub fn new(path: Utf8PathBuf) -> Self {
        Self { path }
    }
}
