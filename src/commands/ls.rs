use surrealdb::Connection;

use surrealfs::{Entry, FsError, SurrealFs};

use super::ReplState;
use super::util::resolve_cli_path;

#[derive(Debug, Clone, Copy)]
struct LsOptions {
    all: bool,
    long: bool,
    recursive: bool,
    dir_only: bool,
    human: bool,
}

pub async fn run<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    let (opts, target_arg) = parse_ls_args(args);
    let target_path = match target_arg {
        Some(arg) => resolve_cli_path(&state.cwd, arg),
        None => state.cwd.clone(),
    };

    handle_ls(&state.fs, &target_path, opts).await
}

fn parse_ls_args<'a>(args: &'a [&str]) -> (LsOptions, Option<&'a str>) {
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

    (opts, path)
}

async fn handle_ls<DB>(fs: &SurrealFs<DB>, path: &str, opts: LsOptions) -> Result<(), FsError>
where
    DB: Connection,
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
        let size = entry.size();
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
