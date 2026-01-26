use std::io::Write;

use surrealdb::Connection;
use tokio::io::{self, AsyncBufReadExt, BufReader};

use surrealfs::SurrealFs;

use crate::commands::{self, ReplControl, ReplState};

pub async fn run<DB>(fs: SurrealFs<DB>) -> surrealfs::Result<()>
where
    DB: Connection,
{
    println!("SurrealFS interactive demo. Type 'help' for commands. Ctrl-D to exit.\n");
    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

    let mut state = ReplState {
        fs,
        cwd: String::from("/"),
    };

    loop {
        print!("surrealfs> ");
        std::io::stdout().flush().ok();

        let Some(line) = (match lines.next_line().await {
            Ok(v) => v,
            Err(e) => {
                println!("Error reading input: {}", e);
                break;
            }
        }) else {
            println!();
            break;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        let result = commands::dispatch(cmd, &args, &mut state).await;

        match result {
            Ok(ReplControl::Continue) => {}
            Ok(ReplControl::Exit) => break,
            Err(e) => println!("Error: {}", e),
        }
    }

    Ok(())
}
