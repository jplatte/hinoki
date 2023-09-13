#[derive(clap::Parser)]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    Build(BuildArgs),
    Serve,
}

#[derive(clap::Parser)]
pub(crate) struct BuildArgs {}
