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
    Serve(ServeArgs),
}

#[derive(clap::Parser)]
pub struct BuildArgs {
    /// Include draft files in the output.
    #[arg(long = "drafts")]
    pub include_drafts: bool,
}

#[derive(clap::Parser)]
pub struct ServeArgs {
    /// Which port to use.
    #[arg(long, short, default_value = "8000")]
    pub port: u16,

    /// Open site in the default browser once it's built.
    #[arg(long)]
    pub open: bool,
}
