use std::{io::ErrorKind, process::ExitCode, sync::atomic::Ordering};

use bumpalo_herd::Herd;
use camino::Utf8Path;
use fs_err as fs;
use tracing::error;

#[cfg(feature = "syntax-highlighting")]
use crate::content::LazySyntaxHighlighter;
use crate::{
    assets::{AssetsProcessor, AssetsProcessorContext},
    config::Config,
    content::{ContentProcessor, ContentProcessorContext},
    template::{context::GlobalContext, load_templates},
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
        let output_dir = self.config.output_dir();
        if let Err(e) = init_output_directory(&output_dir) {
            error!("failed to initialize output directory: {e:#}");
        }

        let output_dir_mgr = OutputDirManager::new(output_dir);

        let (r1, r2) = rayon::join(
            || self.process_content(&output_dir_mgr),
            || self.process_assets(&output_dir_mgr),
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
            (Ok(false), Ok(false)) => ExitCode::SUCCESS,
            (Ok(_), Ok(_)) => ExitCode::FAILURE,
        }
    }

    fn process_content(&self, output_dir_mgr: &OutputDirManager) -> anyhow::Result<bool> {
        let alloc = Herd::new();
        let template_env = load_templates(&self.config.template_dir(), &alloc)?;
        let cx = ContentProcessorContext::new(
            &self.config,
            self.include_drafts,
            template_env,
            output_dir_mgr,
            GlobalContext::new(
                #[cfg(feature = "syntax-highlighting")]
                &self.config,
                #[cfg(feature = "syntax-highlighting")]
                self.syntax_highlighter.clone(),
            ),
        );
        rayon::scope(|scope| ContentProcessor::new(scope, &cx).run())?;
        Ok(cx.did_error.load(Ordering::Relaxed))
    }

    fn process_assets(&self, output_dir_mgr: &OutputDirManager) -> anyhow::Result<bool> {
        let cx = AssetsProcessorContext::new(&self.config, output_dir_mgr);
        rayon::scope(|scope| AssetsProcessor::new(scope, &cx).run())?;
        Ok(cx.did_error.load(Ordering::Relaxed))
    }
}

pub fn build(config: Config, include_drafts: bool) -> ExitCode {
    Build::new(config, include_drafts).run()
}

pub fn dump(config: Config) -> ExitCode {
    let output_dir_mgr = OutputDirManager::new("".into());
    let cx = ContentProcessorContext::new(
        &config,
        true,
        minijinja::Environment::empty(),
        &output_dir_mgr,
        GlobalContext::new(
            #[cfg(feature = "syntax-highlighting")]
            &config,
            #[cfg(feature = "syntax-highlighting")]
            LazySyntaxHighlighter::default(),
        ),
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

fn init_output_directory(output_dir: &Utf8Path) -> anyhow::Result<()> {
    let read_dir = match fs::read_dir(output_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(output_dir)?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    for entry in read_dir {
        let path = entry?.path();
        fs::remove_dir_all(&path).or_else(|e| {
            if e.kind() == ErrorKind::NotADirectory {
                fs::remove_file(&path)
            } else {
                Err(e)
            }
        })?;
    }

    Ok(())
}
