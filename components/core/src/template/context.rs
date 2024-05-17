use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::{
    de::{self, IntoDeserializer as _},
    Deserialize, Serialize, Serializer,
};
use tracing::warn;

use crate::{
    content::{DirectoryMetadata, FileMetadata},
    util::OrderBiMap,
};

#[derive(Debug)]
pub(crate) struct HinokiContext {
    #[cfg(feature = "syntax-highlighting")]
    pub syntax_highlight_theme: Option<String>,
    pub current_dir_files: Arc<OnceLock<Vec<FileMetadata>>>,
    pub current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
    pub current_file_idx: usize,

    file_indices_by_date: OnceLock<OrderBiMap>,
}

impl HinokiContext {
    pub(crate) fn new(
        current_dir_files: Arc<OnceLock<Vec<FileMetadata>>>,
        current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
        current_file_idx: usize,
    ) -> Self {
        Self {
            syntax_highlight_theme: None,
            current_dir_files,
            current_dir_subdirs,
            current_file_idx,
            file_indices_by_date: OnceLock::new(),
        }
    }

    pub(super) fn current_dir_files(&self) -> &[FileMetadata] {
        loop {
            if let Some(initialized) = self.current_dir_files.get() {
                return initialized;
            }

            if rayon::yield_now().unwrap() == rayon::Yield::Idle {
                warn!("No available work");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub(super) fn get_or_init_file_indices_by(
        &self,
        ordering: Ordering,
        current_dir_files: &[FileMetadata],
    ) -> &OrderBiMap {
        match ordering {
            Ordering::Date => self
                .file_indices_by_date
                .get_or_init(|| OrderBiMap::new(current_dir_files, |file| file.date)),
        }
    }
}

impl minijinja::value::Object for HinokiContext {
    fn repr(self: &Arc<Self>) -> minijinja::value::ObjectRepr {
        minijinja::value::ObjectRepr::Plain
    }
}

#[derive(Serialize)]
pub(crate) struct TemplateContext<'a> {
    pub content: String,
    pub page: &'a FileMetadata,
    pub config: minijinja::Value,
    #[serde(rename = "$hinoki_cx", serialize_with = "serialize_hinoki_cx")]
    pub hinoki_cx: Arc<HinokiContext>,
}

fn serialize_hinoki_cx<S: Serializer>(
    cx: &Arc<HinokiContext>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    minijinja::Value::from(minijinja::value::DynObject::new(cx.clone())).serialize(serializer)
}

pub(super) trait MinijinjaStateExt {
    fn hinoki_cx(&self) -> Result<Arc<HinokiContext>, minijinja::Error>;
}

impl MinijinjaStateExt for minijinja::State<'_, '_> {
    fn hinoki_cx(&self) -> Result<Arc<HinokiContext>, minijinja::Error> {
        self.lookup("$hinoki_cx")
            .ok_or_else(|| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    "internal error: $hinoki_cx is missing",
                )
            })?
            .downcast_object()
            .ok_or_else(|| {
                minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    "internal error: $hinoki_cx type mismatch",
                )
            })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Ordering {
    Date,
}

impl Ordering {
    pub(super) fn from_string(s: &str) -> Result<Self, minijinja::Error> {
        Self::deserialize(s.into_deserializer()).map_err(|e: de::value::Error| {
            minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, e.to_string())
        })
    }
}
