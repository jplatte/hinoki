#[derive(clap::Parser)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Build(BuildArgs),
    DumpMetadata,
    Serve,
}

#[derive(clap::Parser)]
pub struct BuildArgs {
    /// Include draft files in the output.
    #[arg(long = "drafts")]
    pub include_drafts: bool,
}
