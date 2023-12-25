use std::{process::ExitCode, sync::atomic::Ordering};

use bumpalo_herd::Herd;
use tracing::error;

use crate::{
    assets::AssetsProcessorContext,
    config::Config,
    content::{ContentProcessor, ContentProcessorContext},
    template::load_templates,
};

mod output_dir;

pub(crate) use self::output_dir::OutputDirManager;

pub fn build(config: Config, include_drafts: bool) -> ExitCode {
    fn process_content(
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

    fn process_assets(output_dir_mgr: &OutputDirManager) -> anyhow::Result<bool> {
        let ctx = AssetsProcessorContext::new(output_dir_mgr);
        Ok(ctx.did_error.load(Ordering::Relaxed))
    }

    let output_dir_mgr = OutputDirManager::new(config.output_dir.clone());

    let (r1, r2) = rayon::join(
        || process_content(config, include_drafts, &output_dir_mgr),
        || process_assets(&output_dir_mgr),
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
