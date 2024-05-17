use std::{process::ExitCode, sync::atomic::Ordering};

use anyhow::Context as _;
use bumpalo_herd::Herd;
use camino::Utf8Path;
use fs_err::{self as fs};
use rayon::iter::{ParallelBridge as _, ParallelIterator as _};
use tracing::{error, warn};
use walkdir::WalkDir;

#[cfg(feature = "syntax-highlighting")]
use crate::content::LazySyntaxHighlighter;
use crate::{
    config::Config,
    content::{ContentProcessor, ContentProcessorContext},
    template::load_templates,
};

mod output_dir;

pub(crate) use self::output_dir::OutputDirManager;

pub struct Build {
    config: Config,
    include_drafts: bool,
    #[cfg(feature = "syntax-highlighting")]
    syntax_highlighter: LazySyntaxHighlighter,
}

impl Build {
    pub fn new(config: Config, include_drafts: bool) -> Self {
        Self {
            config,
            include_drafts,
            #[cfg(feature = "syntax-highlighting")]
            syntax_highlighter: LazySyntaxHighlighter::default(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn run(&self) -> ExitCode {
        fn copy_assets(output_dir_mgr: &OutputDirManager) -> anyhow::Result<()> {
            WalkDir::new("theme/assets/").into_iter().par_bridge().try_for_each(|entry| {
                let entry = entry.context("walking asset directory")?;
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

                fs::copy(utf8_path, output_path).context("copying asset")?;
                Ok(())
            })
        }

        let output_dir_mgr = OutputDirManager::new(self.config.output_dir.clone());

        let (r1, r2) =
            rayon::join(|| self.run_inner(&output_dir_mgr), || copy_assets(&output_dir_mgr));

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

    fn run_inner(&self, output_dir_mgr: &OutputDirManager) -> anyhow::Result<bool> {
        let alloc = Herd::new();
        let template_env = load_templates(&alloc)?;
        let cx = ContentProcessorContext::new(
            &self.config,
            self.include_drafts,
            template_env,
            output_dir_mgr,
            #[cfg(feature = "syntax-highlighting")]
            self.syntax_highlighter.clone(),
        );
        rayon::scope(|scope| ContentProcessor::new(scope, &cx).run())?;
        Ok(cx.did_error.load(Ordering::Relaxed))
    }
}

pub fn build(config: Config, include_drafts: bool) -> ExitCode {
    Build::new(config, include_drafts).run()
}

pub fn dump(config: Config) -> ExitCode {
    let output_dir_mgr = OutputDirManager::new("".into());
    #[cfg(feature = "syntax-highlighting")]
    let cx = ContentProcessorContext::new(
        &config,
        true,
        minijinja::Environment::empty(),
        &output_dir_mgr,
        #[cfg(feature = "syntax-highlighting")]
        LazySyntaxHighlighter::default(),
    );

    let res = rayon::scope(|scope| ContentProcessor::new(scope, &cx).dump());
    assert!(!cx.did_error.load(Ordering::Relaxed));

    match res {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}
