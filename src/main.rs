use std::process::ExitCode;

use clap::Parser as _;
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

mod cli;

use self::cli::{CliArgs, Command};

use hinoki_core::{
    content::{build, dump},
    read_config,
};

fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hinoki=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = CliArgs::parse();
    let config = match read_config() {
        Ok(c) => c,
        Err(e) => {
            error!("{e:#}");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Build(args) => build(config, args.include_drafts),
        Command::DumpMetadata => dump(config),
        Command::Serve => unimplemented!(),
    }
}
