use clap::Parser as _;
use fs_err as fs;
use tracing::warn;

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

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args = CliArgs::parse();
    let config = match fs::read_to_string("config.toml") {
        Ok(config_str) => toml::from_str(&config_str)?,
        Err(e) => {
            warn!("Failed to open `config.toml`, falling back to defaults. Error: {e}");
            Config::default()
        }
    };

    match args.command {
        Command::Build(args) => build(args, config)?,
        Command::DumpMetadata => dump(config)?,
        Command::Serve => unimplemented!(),
    }

    Ok(())
}
