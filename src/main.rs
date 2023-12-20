use std::process::ExitCode;

use clap::Parser as _;
use hinoki_core::{
    build::{build, dump},
    read_config,
};
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

mod cli;

use self::cli::{CliArgs, Command};

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
        #[cfg(feature = "dev-server")]
        Command::Serve => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed building the Runtime")
            .block_on(hinoki_dev_server::serve(config)),
        #[cfg(not(feature = "dev-server"))]
        Command::Serve => {
            error!(
                "hinoki was compiled without support for this command.\
                 Please recompile with the 'dev-server' feature enabled."
            );
            ExitCode::FAILURE
        }
    }
}
