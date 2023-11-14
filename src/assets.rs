use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::{ParallelBridge, ParallelIterator};
use tracing::{error, warn};
use walkdir::WalkDir;

use crate::build::BuildDirManager;

#[cfg(feature = "sass")]
mod sass;

pub(crate) struct AssetsProcessor<'c, 's, 'sc> {
    render_scope: &'s rayon::Scope<'sc>,
    ctx: &'c AssetsProcessorContext<'c>,
}

impl<'c: 'sc, 's, 'sc> AssetsProcessor<'c, 's, 'sc> {
    pub fn new(render_scope: &'s rayon::Scope<'sc>, ctx: &'c AssetsProcessorContext<'c>) -> Self {
        Self { render_scope, ctx }
    }

    pub fn run(&self) -> anyhow::Result<()> {
        //self.process_content_dir("content/".into(), WriteOutput::Yes)?;
        self.copy_files()?;
        Ok(())
    }

    fn blah() -> anyhow::Result<()> {
        Ok(())
    }

    fn render_file(&self) -> anyhow::Result<()> {
        let ctx = self.ctx;
        let span = tracing::Span::current();

        self.render_scope.spawn(move |_| {
            let _guard = span.enter();

            if let Err(e) = Self::blah() {
                error!("{e:#}");
                ctx.did_error.store(true, Ordering::Relaxed);
            }
        });

        Ok(())
    }

    fn copy_files(&self) -> anyhow::Result<()> {
        WalkDir::new("theme/assets/").into_iter().par_bridge().try_for_each(|entry| {
            let entry = entry?;
            if entry.file_type().is_dir() {
                return Ok(());
            }

            let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
                warn!("Skipping non-utf8 file `{}`", entry.path().display());
                return Ok(());
            };

            let rel_path =
                utf8_path.strip_prefix("theme/assets/").context("invalid WalkDir item")?;
            let output_path = self.ctx.build_dir_mgr.output_path(rel_path, utf8_path)?;

            fs::copy(utf8_path, output_path)?;
            Ok(())
        })
    }
}

pub(crate) struct AssetsProcessorContext<'a> {
    // #[cfg(feature = "sass")]
    // something: somthing...,
    build_dir_mgr: &'a BuildDirManager,
    did_error: AtomicBool,
}

impl<'a> AssetsProcessorContext<'a> {
    pub fn new(build_dir_mgr: &'a BuildDirManager) -> Self {
        Self { build_dir_mgr, did_error: AtomicBool::new(false) }
    }

    fn output_path(
        &self,
        file_path: &Utf8Path,
        assets_path: &Utf8Path,
    ) -> anyhow::Result<Utf8PathBuf> {
        self.build_dir_mgr.output_path(file_path, assets_path)
    }
}
