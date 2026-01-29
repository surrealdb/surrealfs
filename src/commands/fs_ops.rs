use std::path::PathBuf;

use regex::Regex;
use surrealdb::Connection;
use tokio::{fs, fs::OpenOptions, io::AsyncWriteExt};

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
            let src_is_host = src.starts_with("host:");
            let dest_is_host = dest.starts_with("host:");

            if src_is_host && dest_is_host {
                return Err(FsError::InvalidPath);
            }

            if src_is_host {
                let host_path = &src[5..];
                let data = fs::read(host_path)
                    .await
                    .map_err(|e| FsError::Http(format!("read host {}: {}", host_path, e)))?;
                let dest = resolve_cli_path(&state.cwd, dest);
                state.fs.write_bytes(&dest, data).await
            } else if dest_is_host {
                let src = resolve_cli_path(&state.cwd, src);
                let bytes = state.fs.cat_bytes(&src).await?;
                let host_path = &dest[5..];
                let host_pathbuf = PathBuf::from(host_path);

                if let Some(parent) = host_pathbuf.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent).await.map_err(|e| {
                            FsError::Http(format!("create host dir {}: {}", parent.display(), e))
                        })?;
                    }
                }

                if fs::metadata(&host_pathbuf).await.is_ok() {
                    return Err(FsError::AlreadyExists(host_path.to_string()));
                }

                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&host_pathbuf)
                    .await
                    .map_err(|e| {
                        FsError::Http(format!("open host {}: {}", host_pathbuf.display(), e))
                    })?;
                file.write_all(&bytes).await.map_err(|e| {
                    FsError::Http(format!("write host {}: {}", host_pathbuf.display(), e))
                })?;
                Ok(())
            } else {
                let src = resolve_cli_path(&state.cwd, src);
                let dest = resolve_cli_path(&state.cwd, dest);
                state.fs.cp(&src, &dest).await
            }
        }
        _ => Err(help_error()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use surrealdb::Surreal;
    use surrealdb::engine::local::{Db, Mem};

    async fn setup_state() -> ReplState<Db> {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        ReplState {
            fs: surrealfs::SurrealFs::new(db),
            cwd: "/".to_string(),
        }
    }

    fn unique_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros();
        p.push(format!("surrealfs-{}-{}", name, ts));
        p
    }

    #[tokio::test]
    async fn cp_host_to_virtual() {
        let host_dir = unique_path("host-src");
        fs::create_dir_all(&host_dir).await.unwrap();
        let host_file = host_dir.join("file.bin");
        let data = vec![1u8, 2, 3, 4];
        fs::write(&host_file, &data).await.unwrap();

        let mut state = setup_state().await;
        let host_arg = format!("host:{}", host_file.display());
        cp(&[host_arg.as_str(), "/virtual.bin"], &mut state)
            .await
            .unwrap();

        let stored = state.fs.cat_bytes("/virtual.bin").await.unwrap();
        assert_eq!(stored, data);

        fs::remove_dir_all(&host_dir).await.unwrap();
    }

    #[tokio::test]
    async fn cp_virtual_to_host_respects_existing_and_creates_parent() {
        let mut state = setup_state().await;
        state
            .fs
            .write_bytes("/data.bin", vec![9u8, 8, 7])
            .await
            .unwrap();

        let host_dir = unique_path("host-dest");
        let host_file = host_dir.join("out.bin");
        let host_arg = format!("host:{}", host_file.display());

        // creates parent automatically
        cp(&["/data.bin", host_arg.as_str()], &mut state)
            .await
            .unwrap();
        let read_back = fs::read(&host_file).await.unwrap();
        assert_eq!(read_back, vec![9u8, 8, 7]);

        // second copy should fail due to existing target
        let err = cp(&["/data.bin", host_arg.as_str()], &mut state)
            .await
            .unwrap_err();
        matches!(err, FsError::AlreadyExists(_));

        fs::remove_dir_all(&host_dir).await.unwrap();
    }
}
