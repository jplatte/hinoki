use std::collections::BTreeMap;

use camino::Utf8PathBuf;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::ProcessContent;

#[derive(Debug)]
pub(crate) struct DirectoryMetadata {
    pub subdirs: BTreeMap<String, DirectoryMetadata>,
    pub pages: Vec<PageMetadata>,
    pub assets: Vec<AssetMetadata>,
}

#[derive(Debug)]
pub(crate) enum FileMetadata {
    Page(PageMetadata),
    Asset(AssetMetadata),
}

#[derive(Debug, Serialize)]
pub(crate) struct PageMetadata {
    pub draft: bool,
    pub slug: String,
    pub path: Utf8PathBuf,
    pub title: Option<String>,
    pub date: Option<DateTime<Utc>>,

    // further data from frontmatter that should be printed in dump-metadata
    // but not passed to the template as `page.*`
    #[serde(skip)]
    pub template: Utf8PathBuf,
    #[serde(skip)]
    pub process_content: Option<ProcessContent>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AssetMetadata {
    pub path: Utf8PathBuf,
}

impl AssetMetadata {
    pub fn new(path: Utf8PathBuf) -> Self {
        Self { path }
    }
}
