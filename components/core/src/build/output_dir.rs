use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, RwLock},
};

use anyhow::{Context as _, bail};
use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;

pub(crate) struct OutputDirManager {
    /// The path to the output directory
    pub(crate) output_dir: Utf8PathBuf,

    /// Set of output directories created in the build process.
    ///
    /// Used to avoid redundant syscalls for creating already-existing
    /// directories.
    pub(crate) output_subdirs: RwLock<HashSet<Utf8PathBuf>>,

    /// Set of output files mapped to the path of the corresponding content
    /// file.
    ///
    /// Used to detect conflicts between multiple content files wanting to
    /// write the same output.
    pub(crate) output_files: Mutex<HashMap<Utf8PathBuf, Utf8PathBuf>>,
}

impl OutputDirManager {
    pub(crate) fn new(output_dir: Utf8PathBuf) -> Self {
        Self { output_dir, output_subdirs: Default::default(), output_files: Default::default() }
    }

    pub(crate) fn output_path(
        &self,
        output_rel_path: &Utf8Path,
        source_path: &Utf8Path,
    ) -> anyhow::Result<Utf8PathBuf> {
        let mut output_path = self.output_dir.join(output_rel_path);
        if output_path.as_str().ends_with("/") {
            output_path.push("index.html");
        }

        let dir = output_path.parent().unwrap();

        // This is racy, but that's okay.
        let dir_exists = self.output_subdirs.read().unwrap().contains(dir);
        if !dir_exists {
            fs::create_dir_all(output_path.parent().unwrap()).context("creating output subdir")?;
            self.output_subdirs.write().unwrap().insert(dir.to_owned());
        }

        let result = self
            .output_files
            .lock()
            .unwrap()
            .insert(output_path.to_owned(), source_path.to_owned());

        if let Some(other_source_path) = result {
            bail!(
                "Path conflict: `{source_path}` and `{other_source_path}`
                 both map to `{output_path}`"
            );
        }

        Ok(output_path)
    }
}
