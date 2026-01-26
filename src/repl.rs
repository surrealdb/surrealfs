use std::io::Write;

use surrealdb::Connection;
use tokio::io::{self, AsyncBufReadExt, BufReader};

use surrealfs::SurrealFs;

use crate::commands::curl;
use crate::commands::util::{help_error, resolve_cli_path};
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

        if let Some((left, right)) = line.split_once('|') {
            let mut parts = left.trim().split_whitespace();
            let cmd = parts.next().unwrap_or("");
            let args: Vec<&str> = parts.collect();
            let right = right.trim();

            if cmd != "curl" {
                println!("Error: piping is currently supported as 'curl ... | write_file <path>'");
                continue;
            }

            match curl::run_capture(&args, &mut state).await {
                Ok(resp) => {
                    let mut sink_parts = right.split_whitespace();
                    let sink_cmd = sink_parts.next().unwrap_or("");
                    let sink_args: Vec<&str> = sink_parts.collect();

                    match (sink_cmd, sink_args.as_slice()) {
                        ("write_file", [path]) => {
                            let target = resolve_cli_path(&state.cwd, path);
                            match state.fs.write_file(&target, resp.body).await {
                                Ok(()) => println!("Saved to {} (status {})", target, resp.status),
                                Err(e) => println!("Error: {}", e),
                            }
                        }
                        _ => {
                            println!(
                                "Error: piping is currently supported as 'curl ... | write_file <path>'"
                            );
                        }
                    }
                }
                Err(e) => println!("Error: {}", e),
            }

            continue;
        }

        let (cmd_part, redirect) = if let Some((left, right)) = line.split_once('>') {
            (left.trim(), Some(right.trim()))
        } else {
            (line, None)
        };

        let mut parts = cmd_part.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        if let Some(path) = redirect {
            if cmd == "curl" {
                if path.is_empty() {
                    println!("Error: {}", help_error());
                    continue;
                }

                let target = resolve_cli_path(&state.cwd, path);

                match curl::run_capture(&args, &mut state).await {
                    Ok(resp) => match state.fs.write_file(&target, resp.body).await {
                        Ok(()) => println!("Saved to {} (status {})", target, resp.status),
                        Err(e) => println!("Error: {}", e),
                    },
                    Err(e) => println!("Error: {}", e),
                }

                continue;
            } else {
                println!("Error: piping with '>' is supported only for curl");
                continue;
            }
        }

        let result = commands::dispatch(cmd, &args, &mut state).await;

        match result {
            Ok(ReplControl::Continue) => {}
            Ok(ReplControl::Exit) => break,
            Err(e) => println!("Error: {}", e),
        }
    }

    Ok(())
}
