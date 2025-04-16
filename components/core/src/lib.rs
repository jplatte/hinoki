use std::io;

use anyhow::{Context as _, anyhow};
use camino::Utf8Path;
use fs_err as fs;
use tracing::warn;

pub mod build;
pub mod config;
mod content;
mod frontmatter;
mod metadata;
mod template;
mod util;

pub use self::config::Config;

pub fn read_config(path: &Utf8Path) -> anyhow::Result<Config> {
    let mut config = match fs::read_to_string(path) {
        Ok(config_str) => {
            toml::from_str(&config_str).with_context(|| format!("Failed to parse `{path}`"))?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound && path == "config.toml" => {
            warn!("`{path}` not found, falling back to defaults");
            Config::default()
        }
        Err(e) => {
            return Err(anyhow!(e).context(format!("Failed to open `{path}`")));
        }
    };

    config.path = path.to_owned();
    Ok(config)
}
