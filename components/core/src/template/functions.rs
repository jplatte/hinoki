//! Extra functions available to templates, in addition to MiniJinja's builtin
//! functions.

use std::fmt::Display;

use camino::Utf8PathBuf;
use fs_err as fs;
use minijinja::{value::Kwargs, ErrorKind, Value};

use super::context::{HinokiContext, MinijinjaStateExt, Ordering};

pub(super) fn get_file(
    state: &minijinja::State,
    kwargs: Kwargs,
) -> Result<Value, minijinja::Error> {
    let prev_by: Option<String> = kwargs.get("prev_by")?;
    let next_by: Option<String> = kwargs.get("next_by")?;
    kwargs.assert_all_used()?;

    let cx = state.hinoki_cx()?;
    let Some(current_file_idx) = cx.current_file_idx() else {
        return Err(minijinja::Error::new(
            ErrorKind::InvalidOperation,
            "get_file can't be used in repeat",
        ));
    };
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
            Ok(prev_next_by_impl(current_file_idx, |i| i.checked_sub(1), prev_by, &cx).into())
        }
        (None, Some(next_by)) => {
            let next_by = Ordering::from_string(&next_by)?;
            Ok(prev_next_by_impl(current_file_idx, |i| i.checked_add(1), next_by, &cx).into())
        }
    }
}

fn prev_next_by_impl(
    current_file_idx: usize,
    make_adjacent_idx: impl FnOnce(usize) -> Option<usize>,
    ordering: Ordering,
    cx: &HinokiContext,
) -> Option<Value> {
    let current_dir_files = cx.current_dir_files();
    let order_bi_map = cx.get_or_init_file_indices_by(ordering, current_dir_files);
    let self_idx_ordered = order_bi_map.original_to_ordered[current_file_idx];

    let adj_idx = make_adjacent_idx(self_idx_ordered)?;
    let adj_idx_original = *order_bi_map.ordered_to_original.get(adj_idx)?;
    Some(Value::from_serialize(&current_dir_files[adj_idx_original]))
}

pub(super) fn get_files(
    state: &minijinja::State,
    subdir_name: &str,
) -> Result<Value, minijinja::Error> {
    let cx = state.hinoki_cx()?;
    match cx.get_subdir(subdir_name) {
        Some(subdir_meta) => Ok(Value::from_serialize(subdir_meta.files.get().unwrap())),
        None => Err(minijinja::Error::new(
            minijinja::ErrorKind::InvalidOperation,
            format!("no subdirectory `{subdir_name}`"),
        )),
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
        Some(ext) => {
            return Err(make_error(format!(
                "Unsupported file extension `{ext}`. \
                 Only .toml, .json and .yaml / .yml files can be loaded.",
            )));
        }
        None => {
            return Err(make_error("File must have an extension"));
        }
    };

    let file_contents = fs::read_to_string(path).map_err(make_error)?;
    deserialize(&file_contents)
}
