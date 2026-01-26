use std::env;
use std::io::Write;
use std::path::PathBuf;

use regex::Regex;
use reqwest::{Client, Url};
use surrealdb::Surreal;
use surrealdb::engine::any::connect;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::auth::Root;
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

    let mut cwd = String::from("/");

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
                let (opts, target_arg) = parse_ls_args(&args);
                let target_path = match target_arg {
                    Some(arg) => resolve_cli_path(&cwd, arg),
                    None => cwd.clone(),
                };
                handle_ls(&fs, &target_path, opts).await
            }
            "cat" => match args.as_slice() {
                [path] => fs.cat(&resolve_cli_path(&cwd, path)).await.map(|c| {
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
                    let path = resolve_cli_path(&cwd, path);
                    fs.tail(&path, n).await.map(|lines| {
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
                    let path = resolve_cli_path(&cwd, args[0]);
                    let start = args
                        .get(1)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1);
                    fs.nl(&path, start).await.map(|lines| {
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
                    let path = resolve_cli_path(&cwd, args[1]);
                    match Regex::new(pattern) {
                        Ok(re) => fs.grep(&re, &path, recursive).await.map(|matches| {
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
                [path] => fs.touch(&resolve_cli_path(&cwd, path)).await,
                _ => Err(help_error()),
            },
            "edit" => match args.as_slice() {
                [path, old, new] => {
                    let target = resolve_cli_path(&cwd, path);
                    fs.edit(&target, old, new, false).await.map(|diff| {
                        if !diff.is_empty() {
                            print!("{}", diff);
                        }
                    })
                }
                [path, old, new, flag] => {
                    let target = resolve_cli_path(&cwd, path);
                    let replace_all = matches!(*flag, "true" | "1" | "yes" | "-a" | "--all");
                    fs.edit(&target, old, new, replace_all).await.map(|diff| {
                        if !diff.is_empty() {
                            print!("{}", diff);
                        }
                    })
                }
                _ => Err(help_error()),
            },
            "mkdir" => {
                let mut parents = false;
                let mut targets = Vec::new();
                for arg in &args {
                    if *arg == "-p" {
                        parents = true;
                    } else {
                        targets.push(*arg);
                    }
                }

                match targets.as_slice() {
                    [path] => fs.mkdir(&resolve_cli_path(&cwd, path), parents).await,
                    _ => Err(help_error()),
                }
            }
            "write_file" => {
                if args.len() < 2 {
                    Err(help_error())
                } else {
                    let path = resolve_cli_path(&cwd, args[0]);
                    let content = args[1..].join(" ");
                    fs.write_file(&path, content).await
                }
            }
            "cp" => match args.as_slice() {
                [src, dest] => {
                    let src = resolve_cli_path(&cwd, src);
                    let dest = resolve_cli_path(&cwd, dest);
                    fs.cp(&src, &dest).await
                }
                _ => Err(help_error()),
            },
            "curl" => match parse_curl_args(&args, &cwd) {
                Ok(opts) => run_curl(&fs, opts).await,
                Err(e) => Err(e),
            },
            "pwd" => {
                println!("{}", cwd);
                Ok(())
            }
            "cd" => match args.as_slice() {
                [path] => {
                    let target = resolve_cli_path(&cwd, path);
                    match fs.cd(&cwd, &target).await {
                        Ok(new_cwd) => {
                            cwd = new_cwd;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
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
    println!("  edit <path> <old> <new> [replace_all]");
    println!("  mkdir [-p] <path>");
    println!("  write_file <path> <content>");
    println!("  cp <src> <dest>");
    println!("  curl [options] <url>");
    println!("     options: -o <file>, -O, -L, -H <h:v>, -d <data>, -X <method>");
    println!("  pwd");
    println!("  cd <path>");
    println!("  help");
    println!("  exit | quit");
}

fn resolve_cli_path(cwd: &str, input: &str) -> String {
    if input.starts_with('/') {
        input.to_string()
    } else {
        let mut combined = cwd.to_string();
        if !combined.ends_with('/') {
            combined.push('/');
        }
        combined.push_str(input);
        combined
    }
}

#[derive(Debug)]
struct CurlOptions {
    url: String,
    follow: bool,
    headers: Vec<(String, String)>,
    data: Option<String>,
    method: Option<String>,
    out: Option<String>,
}

fn parse_curl_args(args: &[&str], cwd: &str) -> Result<CurlOptions, FsError> {
    let mut follow = false;
    let mut headers = Vec::new();
    let mut data = None;
    let mut method = None;
    let mut out = None;
    let mut url = None;

    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-L" => {
                follow = true;
                i += 1;
            }
            "-H" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                let h = args[i + 1];
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
                i += 2;
            }
            "-d" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                data = Some(args[i + 1].to_string());
                i += 2;
            }
            "-X" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                method = Some(args[i + 1].to_string());
                i += 2;
            }
            "-o" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                out = Some(resolve_cli_path(cwd, args[i + 1]));
                i += 2;
            }
            "-O" => {
                out = Some(String::new());
                i += 1;
            }
            other => {
                if other.starts_with('-') {
                    return Err(help_error());
                }
                url = Some(other.to_string());
                i += 1;
            }
        }
    }

    let url = url.ok_or_else(help_error)?;
    Ok(CurlOptions {
        url,
        follow,
        headers,
        data,
        method,
        out,
    })
}

async fn run_curl<DB>(fs: &SurrealFs<DB>, opts: CurlOptions) -> Result<(), FsError>
where
    DB: surrealdb::Connection,
{
    let mut client = Client::builder();
    if opts.follow {
        client = client.redirect(reqwest::redirect::Policy::limited(10));
    } else {
        client = client.redirect(reqwest::redirect::Policy::none());
    }
    let client = client.build().map_err(|e| FsError::Http(e.to_string()))?;

    let method = opts
        .method
        .clone()
        .unwrap_or_else(|| if opts.data.is_some() { "POST" } else { "GET" }.to_string());

    let mut req = client.request(method.parse().unwrap_or(reqwest::Method::GET), &opts.url);

    for (k, v) in &opts.headers {
        req = req.header(k, v);
    }

    if let Some(body) = &opts.data {
        req = req.body(body.clone());
    }

    let resp = req.send().await.map_err(|e| FsError::Http(e.to_string()))?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| FsError::Http(e.to_string()))?;

    if let Some(out_path) = &opts.out {
        let target = if out_path.is_empty() {
            derive_out_name(&opts.url)
        } else {
            out_path.clone()
        };
        let content = String::from_utf8_lossy(&bytes).to_string();
        fs.write_file(&target, content).await?;
        println!("Saved to {} (status {})", target, status);
    } else {
        println!("Status: {}", status);
        print!("{}", String::from_utf8_lossy(&bytes));
    }

    if !status.is_success() {
        return Err(FsError::Http(format!("HTTP status {}", status)));
    }

    Ok(())
}

fn derive_out_name(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(seg) = parsed
            .path_segments()
            .and_then(|s| s.filter(|v| !v.is_empty()).last())
        {
            return seg.to_string();
        }
    }
    "index.html".to_string()
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
