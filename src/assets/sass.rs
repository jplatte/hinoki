use anyhow::Context;
use camino::Utf8Path;
use fs_err::{self as fs};
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use rsass::{compile_scss_path, output::Format};
use tracing::warn;
use walkdir::WalkDir;

use crate::build::BuildDirManager;

pub(crate) fn load_sass(build_dir_mgr: &BuildDirManager) -> anyhow::Result<()> {
    WalkDir::new("theme/assets/").into_iter().par_bridge().try_for_each(|entry| {
        let entry = entry?;
        if entry.file_type().is_dir() {
            return Ok(());
        }

        let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
            warn!("Skipping non-utf8 file `{}`", entry.path().display());
            return Ok(());
        };

        let rel_path = utf8_path.strip_prefix("theme/assets/").context("invalid WalkDir item")?;
        let output_path = build_dir_mgr.output_path(rel_path, utf8_path)?;

        let css = compile_scss_path(entry.path(), Format::default())?;
        fs::write(rel_path, css)?;
        Ok(())
    })

    // for entry in WalkDir::new("theme/assets/") {
    //     let css = compile_scss_path(entry?.path(), Format::default())?;
    // }
    // Ok(())
}
