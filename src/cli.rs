#[derive(clap::Parser)]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    Build(BuildArgs),
    DumpMetadata,
    Serve,
}

#[derive(clap::Parser)]
pub(crate) struct BuildArgs {
    /// Include draft files in the output.
    #[arg(long = "drafts")]
    pub include_drafts: bool,
}
