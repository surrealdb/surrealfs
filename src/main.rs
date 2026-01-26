use std::env;
use std::io::Write;
use std::path::PathBuf;

use regex::Regex;
use surrealdb::engine::any::connect;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use tokio::io::{self, AsyncBufReadExt, BufReader};

use surrealfs::{Entry, FsError, SurrealFs};

#[tokio::main]
async fn main() -> surrealfs::Result<()> {
    // Demo using either a file-backed engine (default) or a remote SurrealDB.
    // Set env SURREALFS_REMOTE=1 to use remote at ws://127.0.0.1:8000 with root/root.
    let use_remote = env::var("SURREALFS_REMOTE").is_ok();

    if use_remote {
        println!("Using remote SurrealDB at ws://127.0.0.1:8000 (ns=surrealfs, db=demo)");
        let db = connect("ws://127.0.0.1:8000").await?;
        db.signin(Root {
            username: "root",
            password: "root",
        })
        .await?;
        db.use_ns("surrealfs").use_db("demo").await?;
        let fs = SurrealFs::new(db);
        run_repl(fs).await
    } else {
        println!("Using RocksDB-backed SurrealDB at ./demo-db (ns=surrealfs, db=demo)");
        let db_path = PathBuf::from("./demo-db");
        let db = Surreal::new::<RocksDb>(db_path.as_path()).await?;
        db.use_ns("surrealfs").use_db("demo").await?;
        let fs = SurrealFs::new(db);
        run_repl(fs).await
    }
}

async fn run_repl<DB>(fs: SurrealFs<DB>) -> surrealfs::Result<()>
where
    DB: surrealdb::Connection,
{
    println!("SurrealFS interactive demo. Type 'help' for commands. Ctrl-D to exit.\n");
    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();

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

        let result = match cmd {
            "ls" => {
                let (opts, target_path) = parse_ls_args(&args);
                handle_ls(&fs, target_path, opts).await
            }
            "cat" => match args.as_slice() {
                [path] => fs.cat(path).await.map(|c| {
                    print!("{}", c);
                }),
                _ => Err(help_error()),
            },
            "tail" => {
                if args.is_empty() {
                    Err(help_error())
                } else {
                    let (n, path) = if let Ok(n) = args[0].parse::<usize>() {
                        if let Some(path) = args.get(1) {
                            (n, *path)
                        } else {
                            return Err(help_error());
                        }
                    } else {
                        (10, args[0])
                    };
                    fs.tail(path, n).await.map(|lines| {
                        for l in lines {
                            println!("{}", l);
                        }
                    })
                }
            }
            "nl" => {
                if args.is_empty() {
                    Err(help_error())
                } else {
                    let path = args[0];
                    let start = args.get(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
                    fs.nl(path, start).await.map(|lines| {
                        for l in lines {
                            println!("{:>4}  {}", l.number, l.line);
                        }
                    })
                }
            }
            "grep" => {
                if args.len() < 2 {
                    Err(help_error())
                } else {
                    let recursive = args.contains(&"-r") || args.contains(&"--recursive");
                    let pattern = args[0];
                    let path = args[1];
                    match Regex::new(pattern) {
                        Ok(re) => fs.grep(&re, path, recursive).await.map(|matches| {
                            for m in matches {
                                println!("{}:{}: {}", m.path, m.line_number, m.line);
                            }
                        }),
                        Err(e) => {
                            println!("Invalid regex: {}", e);
                            Ok(())
                        }
                    }
                }
            }
            "touch" => match args.as_slice() {
                [path] => fs.touch(path).await,
                _ => Err(help_error()),
            },
            "mkdir_p" => match args.as_slice() {
                [path] => fs.mkdir_p(path).await,
                _ => Err(help_error()),
            },
            "write_file" => {
                if args.len() < 2 {
                    Err(help_error())
                } else {
                    let path = args[0];
                    let content = args[1..].join(" ");
                    fs.write_file(path, content).await
                }
            }
            "cp" => match args.as_slice() {
                [src, dest] => fs.cp(src, dest).await,
                _ => Err(help_error()),
            },
            "help" => {
                print_help();
                Ok(())
            }
            "exit" | "quit" => break,
            _ => {
                print_help();
                Ok(())
            }
        };

        if let Err(e) = result {
            println!("Error: {}", e);
        }
    }

    Ok(())
}

fn print_help() {
    println!("Commands:");
    println!("  ls [options] [path]");
    println!("     options: -l (long), -a (all), -R (recursive), -d (dir only), -h (human sizes)");
    println!("  cat <path>");
    println!("  tail [n] <path>");
    println!("  nl <path> [start]");
    println!("  grep [-r|--recursive] <pattern> <path>");
    println!("  touch <path>");
    println!("  mkdir_p <path>");
    println!("  write_file <path> <content>");
    println!("  cp <src> <dest>");
    println!("  help");
    println!("  exit | quit");
}

fn help_error() -> FsError {
    FsError::InvalidPath
}

#[derive(Debug, Clone, Copy)]
struct LsOptions {
    all: bool,
    long: bool,
    recursive: bool,
    dir_only: bool,
    human: bool,
}

fn parse_ls_args<'a>(args: &'a [&str]) -> (LsOptions, &'a str) {
    let mut opts = LsOptions {
        all: false,
        long: false,
        recursive: false,
        dir_only: false,
        human: false,
    };

    let mut path: Option<&str> = None;

    for &arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg.chars().skip(1) {
                match ch {
                    'a' => opts.all = true,
                    'l' => opts.long = true,
                    'R' => opts.recursive = true,
                    'd' => opts.dir_only = true,
                    'h' => opts.human = true,
                    _ => {}
                }
            }
        } else {
            path = Some(arg);
            break;
        }
    }

    (opts, path.unwrap_or("/"))
}

async fn handle_ls<DB>(fs: &SurrealFs<DB>, path: &str, opts: LsOptions) -> surrealfs::Result<()>
where
    DB: surrealdb::Connection,
{
    if opts.dir_only {
        match fs.ls(path).await {
            Ok(entries) => {
                for e in entries {
                    if e.path == path {
                        print_entry(&e, opts);
                    }
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    } else if opts.recursive {
        let mut stack = vec![path.to_string()];
        while let Some(p) = stack.pop() {
            match fs.ls(&p).await {
                Ok(entries) => {
                    for e in entries.iter() {
                        if !opts.all && e.name.starts_with('.') {
                            continue;
                        }
                        print_entry(e, opts);
                        if e.is_dir {
                            stack.push(e.path.clone());
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    } else {
        match fs.ls(path).await {
            Ok(entries) => {
                for e in entries {
                    if !opts.all && e.name.starts_with('.') {
                        continue;
                    }
                    print_entry(&e, opts);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

fn print_entry(entry: &Entry, opts: LsOptions) {
    if opts.long {
        let kind = if entry.is_dir { 'd' } else { '-' };
        let size = if entry.is_dir {
            0
        } else {
            entry.content.as_ref().map(|c| c.len()).unwrap_or(0)
        };
        if opts.human {
            let (val, unit) = human_size(size as f64);
            println!("{} {:>6.1}{} {}", kind, val, unit, entry.path);
        } else {
            println!("{} {:>8} {}", kind, size, entry.path);
        }
    } else {
        let suffix = if entry.is_dir { "/" } else { "" };
        println!("{}{}", entry.path, suffix);
    }
}

fn human_size(bytes: f64) -> (f64, &'static str) {
    const UNITS: &[&str] = &["B", "K", "M", "G", "T", "P"];
    if bytes < 1.0 {
        return (bytes, "B");
    }
    let mut value = bytes;
    let mut idx = 0;
    while value >= 1024.0 && idx < UNITS.len() - 1 {
        value /= 1024.0;
        idx += 1;
    }
    (value, UNITS[idx])
}
