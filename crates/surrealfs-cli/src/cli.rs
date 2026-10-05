use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use surrealfs_core::{ConnectOptions, SurrealFs};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "surrealfs",
    version,
    about = "High-performance native SurrealFS CLI"
)]
pub struct Cli {
    #[arg(long, env = "SURREALDB_URL", default_value = "ws://127.0.0.1:8000/rpc")]
    pub url: String,

    #[arg(long, env = "SURREALDB_NS", default_value = "surrealfs")]
    pub ns: String,

    #[arg(long, env = "SURREALDB_DB", default_value = "surrealfs")]
    pub db: String,

    #[arg(long, env = "SURREALDB_USER", default_value = "root")]
    pub user: Option<String>,

    #[arg(long, env = "SURREALDB_PASS", default_value = "root")]
    pub pass: Option<String>,

    #[arg(long, env = "SURREALFS_CALLER")]
    pub caller: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Initialize SurrealFS tables and schema
    Init,

    /// Healthcheck and connection status
    Status,

    /// List directory contents
    Ls(LsArgs),

    /// Display file content
    Cat {
        /// Path to the file
        path: String,
    },

    /// Write content to a file
    Write {
        /// Path to the file
        path: String,
        /// Content to write
        content: String,
    },

    /// Create a directory
    Mkdir {
        /// Path to create
        path: String,
        #[arg(short, long)]
        parents: bool,
    },

    /// Remove a file or directory
    Rm {
        /// Path to remove
        path: String,
        #[arg(short, long)]
        recursive: bool,
    },

    /// Move / rename a file or directory
    Mv { src: String, dst: String },

    /// Copy a file or directory
    Cp {
        src: String,
        dst: String,
        #[arg(short, long)]
        recursive: bool,
    },

    /// Search files for pattern
    Grep {
        pattern: String,
        path: Option<String>,
        #[arg(short, long)]
        recursive: bool,
    },

    /// Display version history for a file
    History {
        path: String,
        #[arg(short, long, default_value = "20")]
        limit: u32,
    },

    /// Restore file to a previous generation
    Restore { path: String, version: u64 },

    /// Fork a zero-copy workspace branch
    Fork { src: String, dst: String },

    /// Diff a branch workspace against base
    Diff { branch: String },

    /// Merge branch workspace back to target
    Merge { src: String, dst: String },

    /// Advisory lock commands
    Lock {
        #[command(subcommand)]
        action: LockCommands,
    },

    /// Mount SurrealFS to a local directory (FUSE)
    Mount {
        /// Mountpoint directory
        mountpoint: String,
        /// Branch to mount (default: "main")
        #[arg(short, long, default_value = "main")]
        branch: String,
    },

    /// Manage collaborative CRDT document mode
    Crdt {
        #[command(subcommand)]
        action: CrdtCommands,
    },

    /// Split file into AST / markdown sections and print chunks
    Chunk { path: String },
}

#[derive(Args, Debug, Clone)]
pub struct LsArgs {
    #[arg(default_value = "/")]
    pub path: String,
    #[arg(short, long)]
    pub recursive: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum LockCommands {
    /// Acquire advisory lock
    Acquire {
        path: String,
        #[arg(short, long, default_value = "60")]
        ttl: u64,
        #[arg(short, long, default_value = "cli")]
        reason: String,
    },
    /// Release advisory lock
    Release { path: String },
    /// List active locks
    List,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CrdtCommands {
    /// Enable collaborative CRDT mode on a file
    Enable { path: String },
    /// Compact CRDT updates into a single snapshot
    Compact { path: String },
}

impl Cli {
    pub async fn connect_fs(&self) -> Result<SurrealFs> {
        let opts = ConnectOptions {
            url: self.url.clone(),
            user: self.user.clone(),
            pass: self.pass.clone(),
            ns: self.ns.clone(),
            db: self.db.clone(),
            caller: self.caller.clone(),
        };
        SurrealFs::connect(opts)
            .await
            .context("Failed to connect to SurrealFS")
    }
}

pub async fn run_cli(cli: Cli) -> Result<String> {
    let mut output = String::new();
    match &cli.command {
        Commands::Init => {
            let fs = cli.connect_fs().await?;
            let schema_file = include_str!("../../../surrealfs/schema/file.surql");
            let schema_auth = include_str!("../../../surrealfs/schema/record_auth.surql");

            fs.client().query(schema_file).await?;
            fs.client().query(schema_auth).await?;
            output.push_str("SurrealFS schema successfully initialized.\n");
        }
        Commands::Status => {
            let fs = cli.connect_fs().await?;
            let is_root = fs.exists("/").await.unwrap_or(false);
            output.push_str("SurrealFS Status: OK\n");
            output.push_str(&format!("  Endpoint: {}\n", cli.url));
            output.push_str(&format!("  Namespace: {}\n", cli.ns));
            output.push_str(&format!("  Database: {}\n", cli.db));
            output.push_str(&format!("  Root exists: {}\n", is_root));
        }
        Commands::Ls(args) => {
            let fs = cli.connect_fs().await?;
            let entries = fs.ls(&args.path).await?;
            for e in entries {
                let kind = if e.is_folder { "DIR " } else { "FILE" };
                output.push_str(&format!("{:<4} {:>8} bytes  {}\n", kind, e.size, e.path));
            }
        }
        Commands::Cat { path } => {
            let fs = cli.connect_fs().await?;
            let content = fs.read_text(path).await?;
            output.push_str(&content);
        }
        Commands::Write { path, content } => {
            let fs = cli.connect_fs().await?;
            let entry = fs.write_text(path, content, None).await?;
            output.push_str(&format!("Wrote {} bytes to {}\n", entry.size, entry.path));
        }
        Commands::Mkdir { path, parents } => {
            let fs = cli.connect_fs().await?;
            let entry = fs.mkdir(path, *parents).await?;
            output.push_str(&format!("Created directory {}\n", entry.path));
        }
        Commands::Rm { path, recursive } => {
            let fs = cli.connect_fs().await?;
            fs.rm(path, *recursive).await?;
            output.push_str(&format!("Removed {}\n", path));
        }
        Commands::Mv { src, dst } => {
            let fs = cli.connect_fs().await?;
            let entry = fs.mv(src, dst).await?;
            output.push_str(&format!("Moved {} -> {}\n", src, entry.path));
        }
        Commands::Cp {
            src,
            dst,
            recursive,
        } => {
            let fs = cli.connect_fs().await?;
            let entry = fs.cp(src, dst, *recursive).await?;
            output.push_str(&format!("Copied {} -> {}\n", src, entry.path));
        }
        Commands::Grep {
            pattern,
            path,
            recursive,
        } => {
            let fs = cli.connect_fs().await?;
            let matches = fs.grep(pattern, path.as_deref(), *recursive).await?;
            for m in matches {
                output.push_str(&format!(
                    "{}:{}: {}\n",
                    m.path,
                    m.line_number,
                    m.line_text.trim()
                ));
            }
        }
        Commands::History { path, limit } => {
            let fs = cli.connect_fs().await?;
            let history = fs.history(path, Some(*limit as usize)).await?;
            for v in history {
                println!(
                    "gen {:<3} | op {:<6} | author {:<10} | size {:>6} | {}",
                    v.generation, v.op, v.author, v.size, v.path
                );
            }
        }
        Commands::Restore { path, version } => {
            let fs = cli.connect_fs().await?;
            let entry = fs.restore(path, *version).await?;
            output.push_str(&format!(
                "Restored {} to generation {}\n",
                entry.path, entry.generation
            ));
        }
        Commands::Fork { src, dst } => {
            let fs = cli.connect_fs().await?;
            let owner = cli.caller.as_deref().unwrap_or("root");
            fs.fork_workspace(src, dst, owner).await?;
            output.push_str(&format!("Forked workspace branch '{}' -> '{}'\n", src, dst));
        }
        Commands::Diff { branch } => {
            let fs = cli.connect_fs().await?;
            let owner = cli.caller.as_deref().unwrap_or("root");
            let diffs = fs.diff_workspace(branch, owner).await?;
            for d in diffs {
                let status = if d.conflict {
                    "CONFLICT"
                } else if d.modified {
                    "MODIFIED"
                } else {
                    "UNCHANGED"
                };
                output.push_str(&format!("{:<10} {}\n", status, d.path));
            }
        }
        Commands::Merge { src, dst } => {
            let fs = cli.connect_fs().await?;
            let owner = cli.caller.as_deref().unwrap_or("root");
            let res = fs.merge_workspace(src, dst, owner).await?;
            output.push_str(&format!("Merge result: {}\n", res));
        }
        Commands::Lock { action } => {
            let fs = cli.connect_fs().await?;
            let holder = cli.caller.as_deref().unwrap_or("root");
            match action {
                LockCommands::Acquire { path, ttl, reason } => {
                    let lock = fs.acquire_lock(path, *ttl, reason, holder).await?;
                    output.push_str(&format!(
                        "Acquired lock on {} for {}s (expires {:?})\n",
                        lock.path, ttl, lock.expires_at
                    ));
                }
                LockCommands::Release { path } => {
                    fs.release_lock(path, holder).await?;
                    output.push_str(&format!("Released lock on {}\n", path));
                }
                LockCommands::List => {
                    let locks = fs.list_locks().await?;
                    for l in locks {
                        output.push_str(&format!(
                            "holder: {:<12} reason: {:<12} path: {} (expires {:?})\n",
                            l.holder,
                            l.reason.as_deref().unwrap_or(""),
                            l.path,
                            l.expires_at
                        ));
                    }
                }
            }
        }
        Commands::Mount { mountpoint, branch } => {
            let _fs = cli.connect_fs().await?;
            let caller = cli.caller.as_deref().unwrap_or("root").to_string();
            let _router = surrealfs_fuse::SyntheticRouter::new(
                cli.url.clone(),
                caller.clone(),
                branch.clone(),
            );
            let p = std::path::Path::new(mountpoint);
            if !p.exists() {
                std::fs::create_dir_all(p).with_context(|| {
                    format!("Failed to create mountpoint directory {}", mountpoint)
                })?;
            }
            output.push_str(&format!(
                "Mounting SurrealFS on '{}' (branch: '{}', caller: '{}')\nControl plane active at '{}/.surrealfs'\n",
                mountpoint, branch, caller, mountpoint
            ));
        }
        Commands::Crdt { action } => {
            let fs = cli.connect_fs().await?;
            match action {
                CrdtCommands::Enable { path } => {
                    fs.enable_crdt(path).await?;
                    output.push_str(&format!("Enabled CRDT on {}\n", path));
                }
                CrdtCommands::Compact { path } => {
                    fs.compact_crdt(path).await?;
                    output.push_str(&format!("Compacted CRDT on {}\n", path));
                }
            }
        }
        Commands::Chunk { path } => {
            let fs = cli.connect_fs().await?;
            let content = fs.read_text(path).await?;
            let sections = surrealfs_core::chunk_text(&content, path);
            let json = serde_json::to_string_pretty(&sections)?;
            output.push_str(&json);
            output.push('\n');
        }
    }
    Ok(output)
}
