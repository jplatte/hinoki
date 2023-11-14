use std::{io, process::ExitCode};

use anyhow::{anyhow, Context as _};
use clap::Parser as _;
use fs_err as fs;
use tracing::{error, warn};
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

mod cli;
mod config;
mod content;
mod template;
mod util;

use self::{
    cli::{CliArgs, Command},
    config::Config,
    content::{build, dump},
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
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Build(args) => build(args, config),
        Command::DumpMetadata => dump(config),
        Command::Serve => unimplemented!(),
    }
}

fn read_config() -> anyhow::Result<Config> {
    match fs::read_to_string("config.toml") {
        Ok(config_str) => toml::from_str(&config_str).context("Failed to parse `config.toml`"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!("`config.toml` not found, falling back to defaults");
            Ok(Config::default())
        }
        Err(e) => Err(anyhow!(e).context("Failed to open `config.toml`")),
    }
}
