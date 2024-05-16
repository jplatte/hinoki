//! Extra functions available to templates, in addition to MiniJinja's builtin
//! functions.

use std::{
    collections::BTreeMap,
    fmt::Display,
    sync::{Arc, OnceLock},
    time::Duration,
};

use camino::Utf8PathBuf;
use fs_err as fs;
use minijinja::{
    value::{from_args, Kwargs, Object},
    ErrorKind, Value,
};
use serde::{
    de::{self, IntoDeserializer},
    Deserialize,
};
use tracing::warn;

use crate::{
    content::{DirectoryMetadata, FileMetadata},
    util::OrderBiMap,
};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Ordering {
    Date,
}

impl Ordering {
    fn from_string(s: &str) -> Result<Self, minijinja::Error> {
        Self::deserialize(s.into_deserializer()).map_err(|e: de::value::Error| {
            minijinja::Error::new(ErrorKind::InvalidOperation, e.to_string())
        })
    }
}

#[derive(Debug)]
pub(crate) struct GetFile {
    current_dir_files: Arc<OnceLock<Vec<FileMetadata>>>,
    current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
    current_file_idx: usize,

    file_indices_by_date: OnceLock<OrderBiMap>,
}

impl GetFile {
    pub(crate) fn new(
        current_dir_files: Arc<OnceLock<Vec<FileMetadata>>>,
        current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>,
        current_file_idx: usize,
    ) -> Self {
        Self {
            current_dir_files,
            current_dir_subdirs,
            current_file_idx,
            file_indices_by_date: OnceLock::new(),
        }
    }

    fn current_dir_files(&self) -> &[FileMetadata] {
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

    fn get_or_init_file_indices_by(
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

impl Object for GetFile {
    fn call(
        self: &Arc<Self>,
        _state: &minijinja::State,
        args: &[Value],
    ) -> Result<Value, minijinja::Error> {
        // TODO: Add `Option<String>` non-kw parameter to look up specific file
        //       by relative path
        let (kwargs,): (Kwargs,) = from_args(args)?;
        let prev_by: Option<String> = kwargs.get("prev_by")?;
        let next_by: Option<String> = kwargs.get("next_by")?;

        kwargs.assert_all_used()?;
        match (prev_by, next_by) {
            (None, None) => Err(minijinja::Error::new(
                ErrorKind::InvalidOperation,
                "expected keyword argument prev_by or next_by",
            )),
            (Some(_), Some(_)) => Err(minijinja::Error::new(
                ErrorKind::InvalidOperation,
                "prev_by and next_by are mutually exclusive",
            )),
            (Some(prev_by), None) => {
                let prev_by = Ordering::from_string(&prev_by)?;

                let current_dir_files = self.current_dir_files();
                let order_bi_map = self.get_or_init_file_indices_by(prev_by, current_dir_files);
                let self_idx_ordered = order_bi_map.original_to_ordered[self.current_file_idx];
                if self_idx_ordered > 0 {
                    let prev_idx_original = order_bi_map.ordered_to_original[self_idx_ordered - 1];
                    Ok(Value::from_serialize(&current_dir_files[prev_idx_original]))
                } else {
                    Ok(Value::UNDEFINED)
                }
            }
            (None, Some(next_by)) => {
                let next_by = Ordering::from_string(&next_by)?;

                let current_dir_files = self.current_dir_files();
                let order_bi_map = self.get_or_init_file_indices_by(next_by, current_dir_files);
                let self_idx_ordered = order_bi_map.original_to_ordered[self.current_file_idx];
                match order_bi_map.ordered_to_original.get(self_idx_ordered + 1) {
                    Some(&next_idx_original) => {
                        Ok(Value::from_serialize(&current_dir_files[next_idx_original]))
                    }
                    None => Ok(Value::UNDEFINED),
                }
            }
        }
    }
}

#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct GetFiles {
    current_dir_subdirs: BTreeMap<String, DirectoryMetadata>,
}

impl GetFiles {
    pub(crate) fn new(current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>) -> Arc<Self> {
        // SAFETY: GetFiles is a repr(transparent) struct over the map
        unsafe { Arc::from_raw(Arc::into_raw(current_dir_subdirs) as _) }
    }
}

impl Object for GetFiles {
    fn call(
        self: &Arc<Self>,
        _state: &minijinja::State,
        args: &[Value],
    ) -> Result<Value, minijinja::Error> {
        // TODO: split at slash and do nested lookup?

        let (subdir_name,): (&str,) = from_args(args)?;
        match self.current_dir_subdirs.get(subdir_name) {
            Some(subdir_meta) => Ok(Value::from_serialize(subdir_meta.files.get().unwrap())),
            None => Err(minijinja::Error::new(
                minijinja::ErrorKind::InvalidOperation,
                format!("no subdirectory `{subdir_name}`"),
            )),
        }
    }
}

pub(super) fn load_data(path: String) -> Result<Value, minijinja::Error> {
    // FIXME: MiniJinja's ErrorKind type does not have an Other variant,
    // none of the existing variants really match, update when that changes.
    fn make_error(e: impl Display) -> minijinja::Error {
        minijinja::Error::new(ErrorKind::BadInclude, e.to_string())
    }

    let path = Utf8PathBuf::from(path);
    let deserialize: fn(&str) -> Result<Value, minijinja::Error> = match path.extension() {
        Some("toml") => |s| toml::from_str(s).map_err(make_error),
        #[cfg(feature = "json")]
        Some("json") => |s| serde_json::from_str(s).map_err(make_error),
        #[cfg(not(feature = "json"))]
        Some("json") => {
            return Err(make_error(
                "hinoki was compiled without support for JSON files.\
                 Please recompile with the 'json' feature enabled.",
            ));
        }
        #[cfg(feature = "yaml")]
        Some("yml" | "yaml") => |s| serde_yaml::from_str(s).map_err(make_error),
        #[cfg(not(feature = "yaml"))]
        Some("yml" | "yaml") => {
            return Err(make_error(
                "hinoki was compiled without support for YAML files.\
                 Please recompile with the 'yaml' feature enabled.",
            ));
        }
        _ => {
            return Err(make_error(
                "Unsupported file extension.\
                 Only .toml, .json and .yaml / .yml files can be loaded.",
            ));
        }
    };

    let file_contents = fs::read_to_string(path).map_err(make_error)?;
    deserialize(&file_contents)
}
