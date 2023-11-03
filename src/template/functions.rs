//! Extra functions available to templates, in addition to MiniJinja's builtin
//! functions.

use std::{
    collections::BTreeMap,
    fmt::{self, Display},
    sync::Arc,
};

use camino::Utf8PathBuf;
use fs_err as fs;
use minijinja::{
    value::{from_args, Object},
    ErrorKind, Value,
};

use crate::content::DirectoryMetadata;

#[repr(transparent)]
#[derive(Debug)]
pub(crate) struct GetPages {
    current_dir_subdirs: BTreeMap<String, DirectoryMetadata>,
}

impl GetPages {
    pub(crate) fn new(current_dir_subdirs: Arc<BTreeMap<String, DirectoryMetadata>>) -> Arc<Self> {
        // SAFETY: GetPages is a repr(transparent) struct over the map
        unsafe { Arc::from_raw(Arc::into_raw(current_dir_subdirs) as _) }
    }
}

impl fmt::Display for GetPages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "get_pages")
    }
}

impl Object for GetPages {
    fn call(&self, _state: &minijinja::State, args: &[Value]) -> Result<Value, minijinja::Error> {
        // TODO: split at slash and do nested lookup?

        let (subdir_name,): (&str,) = from_args(args)?;
        match self.current_dir_subdirs.get(subdir_name) {
            Some(subdir_meta) => Ok(Value::from_serializable(&subdir_meta.pages)),
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
