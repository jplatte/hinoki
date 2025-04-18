use std::process::ExitCode;

use clap::Parser as _;
use hinoki_core::{
    build::{build, dump},
    read_config,
};
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use hinoki_cli::{CliArgs, Command};

fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hinoki=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = CliArgs::parse();
    let config = match read_config(&args.config) {
        Ok(c) => c,
        Err(e) => {
            error!("{e:#}");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Build(args) => build(config, args.include_drafts),
        Command::DumpMetadata => dump(config),
        #[cfg(feature = "dev-server")]
        Command::Serve(args) => hinoki_dev_server::run(config, args),
        #[cfg(not(feature = "dev-server"))]
        Command::Serve(_) => {
            error!(
                "hinoki was compiled without support for this command.\
                 Please recompile with the 'dev-server' feature enabled."
            );
            ExitCode::FAILURE
        }
    }
}
