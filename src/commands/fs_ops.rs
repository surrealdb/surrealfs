use regex::Regex;
use surrealdb::Connection;

use surrealfs::FsError;

use super::ReplState;
use super::util::{help_error, resolve_cli_path};

pub async fn cat<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [path] => state
            .fs
            .cat(&resolve_cli_path(&state.cwd, path))
            .await
            .map(|c| print!("{}", c)),
        _ => Err(help_error()),
    }
}

pub async fn tail<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
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
        let path = resolve_cli_path(&state.cwd, path);
        state.fs.tail(&path, n).await.map(|lines| {
            for l in lines {
                println!("{}", l);
            }
        })
    }
}

pub async fn read<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [path, offset, limit] => {
            let offset = offset.parse::<usize>().map_err(|_| help_error())?;
            let limit = limit.parse::<usize>().map_err(|_| help_error())?;
            let path = resolve_cli_path(&state.cwd, path);
            state.fs.read(&path, offset, limit).await.map(|lines| {
                for l in lines {
                    println!("{}", l);
                }
            })
        }
        _ => Err(help_error()),
    }
}

pub async fn nl<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    if args.is_empty() {
        Err(help_error())
    } else {
        let path = resolve_cli_path(&state.cwd, args[0]);
        let start = args
            .get(1)
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1);
        state.fs.nl(&path, start).await.map(|lines| {
            for l in lines {
                println!("{:>4}  {}", l.number, l.line);
            }
        })
    }
}

pub async fn grep<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    if args.len() < 2 {
        Err(help_error())
    } else {
        let recursive = args.contains(&"-r") || args.contains(&"--recursive");
        let pattern = args[0];
        let path = resolve_cli_path(&state.cwd, args[1]);
        match Regex::new(pattern) {
            Ok(re) => state.fs.grep(&re, &path, recursive).await.map(|matches| {
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

pub async fn glob<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [pattern] => {
            let pattern = resolve_cli_path(&state.cwd, pattern);
            state.fs.glob(&pattern).await.map(|paths| {
                for p in paths {
                    println!("{}", p);
                }
            })
        }
        _ => Err(help_error()),
    }
}

pub async fn touch<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [path] => state.fs.touch(&resolve_cli_path(&state.cwd, path)).await,
        _ => Err(help_error()),
    }
}

pub async fn edit<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    if args.len() < 3 {
        return Err(help_error());
    }

    let path = resolve_cli_path(&state.cwd, args[0]);
    let old = unquote(args[1]);

    let (new_parts, replace_all) = if args.len() >= 4 {
        let (maybe_flag, rest) = args.split_last().unwrap();
        let is_flag = matches!(*maybe_flag, "true" | "1" | "yes" | "-a" | "--all");

        if is_flag {
            (rest[2..].to_vec(), true)
        } else {
            (args[2..].to_vec(), false)
        }
    } else {
        (args[2..].to_vec(), false)
    };

    let new = unquote(&new_parts.join(" "));

    state
        .fs
        .edit(&path, old.as_str(), new.as_str(), replace_all)
        .await
        .map(|diff| {
            if !diff.is_empty() {
                print!("{}", diff);
            }
        })
}

fn unquote(input: &str) -> String {
    if input.len() >= 2 {
        let bytes = input.as_bytes();
        let first = bytes[0];
        let last = *bytes.last().unwrap();

        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return input[1..input.len() - 1].to_string();
        }
    }

    input.to_string()
}

pub async fn mkdir<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    let mut parents = false;
    let mut targets = Vec::new();
    for arg in args {
        if *arg == "-p" {
            parents = true;
        } else {
            targets.push(*arg);
        }
    }

    match targets.as_slice() {
        [path] => {
            state
                .fs
                .mkdir(&resolve_cli_path(&state.cwd, path), parents)
                .await
        }
        _ => Err(help_error()),
    }
}

pub async fn write_file<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    if args.len() < 2 {
        Err(help_error())
    } else {
        let path = resolve_cli_path(&state.cwd, args[0]);
        let content = args[1..].join(" ");
        state.fs.write_file(&path, content).await
    }
}

pub async fn cp<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    match args {
        [src, dest] => {
            let src = resolve_cli_path(&state.cwd, src);
            let dest = resolve_cli_path(&state.cwd, dest);
            state.fs.cp(&src, &dest).await
        }
        _ => Err(help_error()),
    }
}
