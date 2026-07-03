use anyhow::Result;
use clap::Parser;
use styx_cli::{args::Cli, run};

#[tokio::main]
async fn main() -> Result<()> {
    run(Cli::parse()).await
}
