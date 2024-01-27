use std::{process::ExitCode, sync::atomic::Ordering};

use anyhow::Context as _;
use bumpalo_herd::Herd;
use camino::Utf8Path;
use fs_err::{self as fs};
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use tracing::{error, warn};
use walkdir::WalkDir;

use crate::{
    config::Config,
    content::{ContentProcessor, ContentProcessorContext},
    template::load_templates,
};

mod output_dir;

pub(crate) use self::output_dir::OutputDirManager;

pub fn build(config: Config, include_drafts: bool) -> ExitCode {
    fn build_inner(
        config: Config,
        include_drafts: bool,
        output_dir_mgr: &OutputDirManager,
    ) -> anyhow::Result<bool> {
        let alloc = Herd::new();
        let template_env = load_templates(&alloc)?;
        let ctx =
            ContentProcessorContext::new(config, include_drafts, template_env, output_dir_mgr);
        rayon::scope(|scope| ContentProcessor::new(scope, &ctx).run())?;
        Ok(ctx.did_error.load(Ordering::Relaxed))
    }

    fn copy_assets(output_dir_mgr: &OutputDirManager) -> anyhow::Result<()> {
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
            let output_path = output_dir_mgr.output_path(rel_path, utf8_path)?;

            fs::copy(utf8_path, output_path)?;
            Ok(())
        })
    }

    let output_dir_mgr = OutputDirManager::new(config.output_dir.clone());

    let (r1, r2) = rayon::join(
        || build_inner(config, include_drafts, &output_dir_mgr),
        || copy_assets(&output_dir_mgr),
    );

    match (r1, r2) {
        (Err(e1), Err(e2)) => {
            error!("{e1:#}");
            error!("{e2:#}");
            ExitCode::FAILURE
        }
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
        (Ok(true), Ok(())) => ExitCode::FAILURE,
        (Ok(false), Ok(())) => ExitCode::SUCCESS,
    }
}

pub fn dump(config: Config) -> ExitCode {
    let output_dir_mgr = OutputDirManager::new("".into());
    let ctx = ContentProcessorContext::new(
        config,
        true,
        minijinja::Environment::empty(),
        &output_dir_mgr,
    );

    let res = rayon::scope(|scope| ContentProcessor::new(scope, &ctx).dump());
    assert!(!ctx.did_error.load(Ordering::Relaxed));

    match res {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
