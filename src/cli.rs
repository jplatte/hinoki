use camino::Utf8PathBuf;

#[derive(clap::Parser)]
pub struct CliArgs {
    /// Path to the configuration file.
    #[arg(global = true, long, short, default_value = "config.toml")]
    pub config: Utf8PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Build the site.
    Build(BuildArgs),
    /// Dump site metadata (for debugging purposes).
    DumpMetadata,
    /// Start a development server.
    Serve,
}

#[derive(clap::Parser)]
pub struct BuildArgs {
    /// Include draft files in the output.
    #[arg(long = "drafts")]
    pub include_drafts: bool,
}
