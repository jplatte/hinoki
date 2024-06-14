use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde::{
    de::{self, IntoDeserializer as _},
    Deserialize, Serialize, Serializer,
};
use tracing::warn;

#[cfg(feature = "syntax-highlighting")]
use crate::content::{LazySyntaxHighlighter, SyntaxHighlighter};
use crate::{
    content::{DirectoryMetadata, FileMetadata},
    util::OrderBiMap,
};

#[derive(Clone)]
pub(crate) struct GlobalContext {
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: LazySyntaxHighlighter,
}

impl GlobalContext {
    pub(crate) fn new(
        #[cfg(feature = "syntax-highlighting")] syntax_highlighter: LazySyntaxHighlighter,
    ) -> Self {
        Self { syntax_highlighter }
    }

    #[cfg(feature = "syntax-highlighting")]
    pub(crate) fn syntax_highlighter(&self) -> anyhow::Result<&SyntaxHighlighter> {
        self.syntax_highlighter.get_or_try_init(SyntaxHighlighter::new)
    }
}

#[derive(Clone)]
pub(crate) struct DirectoryContext {
    subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
    files: Arc<OnceLock<Vec<FileMetadata>>>,
    file_indices_by_date: Arc<OnceLock<OrderBiMap>>,
}

impl DirectoryContext {
    pub(crate) fn new(subdirs: Arc<BTreeMap<String, DirectoryMetadata>>) -> Self {
        Self {
            subdirs,
            files: Arc::new(OnceLock::new()),
            file_indices_by_date: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn set_files(&self, files: Vec<FileMetadata>) {
        self.files.set(files).expect("must only be called once")
    }

    pub(crate) fn into_metadata(self) -> DirectoryMetadata {
        DirectoryMetadata { subdirs: self.subdirs, files: self.files }
    }
}

pub(crate) struct RenderContext {
    pub current_file_idx: Option<usize>,
    #[cfg(feature = "syntax-highlighting")]
    pub syntax_highlight_theme: Option<String>,
}

impl RenderContext {
    pub(crate) fn new(
        current_file_idx: Option<usize>,
        #[cfg(feature = "syntax-highlighting")] syntax_highlight_theme: Option<String>,
    ) -> Self {
        Self { syntax_highlight_theme, current_file_idx }
    }
}

pub(crate) struct HinokiContext {
    pub global: GlobalContext,
    pub directory: DirectoryContext,
    pub render: RenderContext,
}

impl HinokiContext {
    pub(crate) fn new(
        global: GlobalContext,
        directory: DirectoryContext,
        render: RenderContext,
    ) -> Arc<Self> {
        Arc::new(Self { global, directory, render })
    }

    #[cfg(feature = "syntax-highlighting")]
    pub(crate) fn syntax_highlighter(&self) -> anyhow::Result<&SyntaxHighlighter> {
        self.global.syntax_highlighter()
    }

    #[cfg(feature = "syntax-highlighting")]
    pub(crate) fn syntax_highlight_theme(&self) -> Option<&str> {
        self.render.syntax_highlight_theme.as_deref()
    }

    pub(super) fn get_subdir(&self, subdir_name: &str) -> Option<&DirectoryMetadata> {
        self.directory.subdirs.get(subdir_name)
    }

    pub(super) fn current_dir_files(&self) -> &[FileMetadata] {
        loop {
            if let Some(initialized) = self.directory.files.get() {
                return initialized;
            }

            if rayon::yield_now().unwrap() == rayon::Yield::Idle {
                warn!("No available work");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub(super) fn current_file_idx(&self) -> Option<usize> {
        self.render.current_file_idx
    }

    pub(super) fn get_or_init_file_indices_by(
        &self,
        ordering: Ordering,
        current_dir_files: &[FileMetadata],
    ) -> &OrderBiMap {
        match ordering {
            Ordering::Date => self
                .directory
                .file_indices_by_date
                .get_or_init(|| OrderBiMap::new(current_dir_files, |file| file.date)),
        }
    }
}

impl fmt::Debug for HinokiContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HinokiContext").finish_non_exhaustive()
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
    pub hinoki_cx: &'a Arc<HinokiContext>,
}

pub(crate) fn serialize_hinoki_cx<S: Serializer>(
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
