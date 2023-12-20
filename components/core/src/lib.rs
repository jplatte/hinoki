use std::io;

use anyhow::{anyhow, Context as _};
use fs_err as fs;
use tracing::warn;

pub mod build;
mod config;
mod content;
mod frontmatter;
mod template;
mod util;

pub use self::config::Config;

pub fn read_config() -> anyhow::Result<Config> {
    match fs::read_to_string("config.toml") {
        Ok(config_str) => toml::from_str(&config_str).context("Failed to parse `config.toml`"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            warn!("`config.toml` not found, falling back to defaults");
            Ok(Config::default())
        }
        Err(e) => Err(anyhow!(e).context("Failed to open `config.toml`")),
    }
}
