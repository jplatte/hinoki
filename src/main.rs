use clap::Parser as _;
use fs_err as fs;
use tracing::warn;

mod build;
mod cli;
mod config;
mod frontmatter;
mod template;

use self::{
    build::build,
    cli::{CliArgs, Command},
    config::Config,
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
        Command::Serve => unimplemented!(),
    }

    Ok(())
}
