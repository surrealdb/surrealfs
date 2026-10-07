//! Git remote helper binary for surrealfs:// URLs (§21.5).

use std::env;
use std::io::{self, BufReader};
use surrealfs_core::GitRemoteHelper;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: git-remote-surrealfs <remote-name> <url>");
        std::process::exit(1);
    }
    let url = &args[2];
    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin.lock());
    let writer = stdout.lock();

    GitRemoteHelper::handle_command(reader, writer, url)?;
    Ok(())
}
