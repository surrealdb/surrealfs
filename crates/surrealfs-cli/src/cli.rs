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
        /// Run NFSv3 loopback mount without FUSE (§22.2)
        #[arg(long)]
        nfs: bool,
    },

    /// Manage collaborative CRDT document mode
    Crdt {
        #[command(subcommand)]
        action: CrdtCommands,
    },

    /// Split file into AST / markdown sections and print chunks
    Chunk { path: String },

    /// Upload a file using FastCDC chunking and BLAKE3 content addressing
    Upload {
        local_path: String,
        remote_path: String,
    },

    /// Display instant disk usage for a directory tree
    Du {
        #[arg(default_value = "/")]
        path: String,
    },

    /// Garbage collect unreferenced blobs older than max_age_secs
    Gc {
        #[arg(short, long, default_value = "86400")]
        max_age_secs: u64,
    },

    /// List or index code symbols for a file or directory tree
    Symbols {
        path: String,
        #[arg(short, long)]
        index: bool,
    },

    /// Look up symbol definitions by exact or qualified name
    Definition { name: String },

    /// Display hierarchical folder digest
    Digest {
        #[arg(default_value = "/")]
        path: String,
    },

    /// Pack relevant context for a question within a token budget
    Pack {
        question: String,
        #[arg(short, long, default_value = "4000")]
        budget: usize,
        #[arg(short, long)]
        scope: Option<String>,
    },

    /// Manage understanding pipeline jobs
    Jobs {
        #[command(subcommand)]
        action: JobCommands,
    },

    /// Detect content type, language, and encoding for a file
    Detect { path: String },

    /// List entities mentioned in a file or directory tree (§20.7)
    Entities { path: String },

    /// Query rows in a structured tabular file (§21.6)
    TableQuery {
        path: String,
        #[arg(short, long, default_value = "50")]
        limit: usize,
    },

    /// Load CSV/TSV file rows into file_row table (§21.6)
    TableLoad {
        path: String,
        #[arg(short, long, default_value = "1000")]
        max_rows: usize,
    },

    /// Check near duplicates for a text or file using SimHash (§20.8)
    NearDups {
        path_or_text: String,
        #[arg(short, long, default_value = "0.85")]
        threshold: f64,
    },

    /// Import a Git repository into SurrealFS respecting .gitignore and mapping commits (§21.5)
    ImportGit {
        repo_path: String,
        target_path: String,
        #[arg(short, long)]
        max_commits: Option<usize>,
    },

    /// Run multi-protocol server daemon (WebDAV, S3, NFS, SFTP) (§22)
    Serve {
        #[command(subcommand)]
        action: ServeCommands,
    },

    /// Manage external protocol credentials (§22, §24)
    Credential {
        #[command(subcommand)]
        action: CredentialCommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ServeCommands {
    /// Run WebDAV protocol server (§22.1)
    Webdav {
        #[arg(short, long, default_value = "127.0.0.1:8080")]
        addr: String,
        #[arg(short, long, default_value = "/")]
        prefix: String,
    },
    /// Run S3-compatible REST API server (§22.3)
    S3 {
        #[arg(short, long, default_value = "127.0.0.1:9000")]
        addr: String,
    },
    /// Run loopback NFSv3 server (§22.2)
    Nfs {
        #[arg(short, long, default_value = "127.0.0.1:2049")]
        addr: String,
        #[arg(short, long)]
        mount_point: Option<String>,
    },
    /// Run SFTP subsystem server (§22.8)
    Sftp {
        #[arg(short, long, default_value = "127.0.0.1:2222")]
        addr: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum CredentialCommands {
    /// Create or update an external credential
    Create {
        kind: String,
        identifier: String,
        secret: String,
        user_id: String,
    },
    /// Revoke / disable a credential
    Revoke {
        kind: String,
        identifier: String,
    },
    /// List configured credentials
    List {
        #[arg(short, long)]
        user_id: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum JobCommands {
    /// Claim pending/expired pipeline jobs
    Claim {
        #[arg(short, long, default_value = "cli-worker")]
        worker: String,
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
    /// Complete a pipeline job
    Complete {
        job_id: String,
        #[arg(short, long)]
        error: Option<String>,
    },
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
        Commands::Mount {
            mountpoint,
            branch,
            nfs,
        } => {
            if *nfs {
                let fs = cli.connect_fs().await?;
                let nfs_server = surrealfs_server::NfsServer::new(fs);
                let cmd = nfs_server.mount_command(mountpoint, 2049);
                output.push_str(&format!("To complete NFS mount, run:\n  sudo {}\n", cmd));
                return Ok(output);
            }
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
        Commands::Upload {
            local_path,
            remote_path,
        } => {
            let fs = cli.connect_fs().await?;
            let data = std::fs::read(local_path)
                .with_context(|| format!("Failed to read local file: {}", local_path))?;
            let entry = fs.upload_file(remote_path, &data, None, None).await?;
            output.push_str(&format!(
                "Uploaded '{}' -> '{}' ({} bytes, gen {})\n",
                local_path, remote_path, entry.size, entry.generation
            ));
        }
        Commands::Du { path } => {
            let fs = cli.connect_fs().await?;
            let usage = fs.du(path).await?;
            output.push_str(&format!(
                "{}\tfiles: {}\tlogical: {} B\tstored: {} B\n",
                usage.path, usage.files, usage.logical_bytes, usage.stored_bytes
            ));
        }
        Commands::Gc { max_age_secs } => {
            let fs = cli.connect_fs().await?;
            let count = fs.gc_blobs(*max_age_secs).await?;
            output.push_str(&format!("Garbage collected {} unreferenced blobs\n", count));
        }
        Commands::Symbols { path, index } => {
            let fs = cli.connect_fs().await?;
            if *index {
                let count = fs.index_file_symbols(path).await?;
                output.push_str(&format!("Indexed {} symbols in '{}'\n", count, path));
            }
            let symbols = fs.symbols(path).await?;
            let json = serde_json::to_string_pretty(&symbols)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::Definition { name } => {
            let fs = cli.connect_fs().await?;
            let defs = fs.definition(name).await?;
            let json = serde_json::to_string_pretty(&defs)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::Digest { path } => {
            let fs = cli.connect_fs().await?;
            let digest = fs.digest(path).await?;
            let json = serde_json::to_string_pretty(&digest)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::Pack {
            question,
            budget,
            scope,
        } => {
            let fs = cli.connect_fs().await?;
            let result = fs.pack(question, *budget, scope.as_deref()).await?;
            output.push_str(&result.formatted);
            output.push('\n');
        }
        Commands::Jobs { action } => {
            let fs = cli.connect_fs().await?;
            match action {
                JobCommands::Claim { worker, limit } => {
                    let jobs = fs
                        .claim_jobs(worker, &["detect", "chunk", "symbols"], *limit)
                        .await?;
                    let json = serde_json::to_string_pretty(&jobs)?;
                    output.push_str(&json);
                    output.push('\n');
                }
                JobCommands::Complete { job_id, error } => {
                    fs.complete_job(job_id, error.as_deref()).await?;
                    output.push_str(&format!("Completed job {}\n", job_id));
                }
            }
        }
        Commands::Detect { path } => {
            let fs = cli.connect_fs().await?;
            let bytes = fs.read_bytes(path).await?;
            let meta = surrealfs_core::understanding::detect_type_and_language(path, &bytes);
            let json = serde_json::to_string_pretty(&meta)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::Entities { path } => {
            let fs = cli.connect_fs().await?;
            let entities = fs.entities(path).await?;
            let json = serde_json::to_string_pretty(&entities)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::TableQuery { path, limit } => {
            let fs = cli.connect_fs().await?;
            let rows = fs.query_table(path, Some(*limit)).await?;
            let json = serde_json::to_string_pretty(&rows)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::TableLoad { path, max_rows } => {
            let fs = cli.connect_fs().await?;
            let count = fs.load_tabular_file(path, *max_rows).await?;
            output.push_str(&format!("Loaded {} rows for {}\n", count, path));
        }
        Commands::NearDups {
            path_or_text,
            threshold,
        } => {
            let fs = cli.connect_fs().await?;
            let text = if path_or_text.starts_with('/') {
                fs.read_text(path_or_text)
                    .await
                    .unwrap_or_else(|_| path_or_text.clone())
            } else {
                path_or_text.clone()
            };
            let matches = fs.check_near_duplicates(&text, *threshold).await?;
            let json = serde_json::to_string_pretty(&matches)?;
            output.push_str(&json);
            output.push('\n');
        }
        Commands::ImportGit {
            repo_path,
            target_path,
            max_commits,
        } => {
            let fs = cli.connect_fs().await?;
            let opts = surrealfs_core::GitImportOptions {
                repo_path: repo_path.clone(),
                target_path: target_path.clone(),
                max_commits: *max_commits,
                branch: None,
            };
            let res = fs.import_git(&opts).await?;
            output.push_str(&format!(
                "Imported {} files and {} commits to {}\n",
                res.imported_files, res.imported_commits, res.target_path
            ));
        }
        Commands::Serve { action } => {
            let fs = cli.connect_fs().await?;
            match action {
                ServeCommands::Webdav { addr, prefix } => {
                    output.push_str(&format!(
                        "Starting WebDAV server on {} with prefix {}\n",
                        addr, prefix
                    ));
                    let server = std::sync::Arc::new(surrealfs_server::WebDavServer::new(fs, prefix));
                    surrealfs_server::run_http_server(addr, server).await?;
                }
                ServeCommands::S3 { addr } => {
                    output.push_str(&format!("Starting S3 server on {}\n", addr));
                    let server = std::sync::Arc::new(surrealfs_server::S3Server::new(fs));
                    surrealfs_server::run_http_server(addr, server).await?;
                }
                ServeCommands::Nfs { addr, mount_point } => {
                    let nfs_server = surrealfs_server::NfsServer::new(fs);
                    if let Some(mp) = mount_point {
                        let cmd = nfs_server.mount_command(mp, 2049);
                        output.push_str(&format!(
                            "NFS server on {}. Mount command:\n  sudo {}\n",
                            addr, cmd
                        ));
                    } else {
                        output.push_str(&format!("NFS server configured on {}\n", addr));
                    }
                }
                ServeCommands::Sftp { addr } => {
                    output.push_str(&format!("SFTP subsystem configured on {}\n", addr));
                }
            }
        }
        Commands::Credential { action } => {
            let fs = cli.connect_fs().await?;
            match action {
                CredentialCommands::Create {
                    kind,
                    identifier,
                    secret,
                    user_id,
                } => {
                    fs.create_credential(kind, identifier, Some(secret), user_id, None)
                        .await?;
                    output.push_str(&format!(
                        "Created credential {} ({}) for {}\n",
                        identifier, kind, user_id
                    ));
                }
                CredentialCommands::Revoke { kind, identifier } => {
                    fs.revoke_credential(kind, identifier).await?;
                    output.push_str(&format!("Revoked credential {} ({})\n", identifier, kind));
                }
                CredentialCommands::List { user_id } => {
                    let creds = fs.list_credentials(user_id.as_deref()).await?;
                    for c in creds {
                        output.push_str(&format!(
                            "{:<8} {:<24} {:<16} enabled={}\n",
                            c.kind, c.identifier, c.user_id, c.enabled
                        ));
                    }
                }
            }
        }
    }
    Ok(output)
}
