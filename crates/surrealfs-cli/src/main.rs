use anyhow::Result;
use clap::Parser;
use surrealfs_cli::{run_cli, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let out = run_cli(cli).await?;
    print!("{}", out);
    Ok(())
}
