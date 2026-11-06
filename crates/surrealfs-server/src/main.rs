#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use std::path::PathBuf;
    use surrealfs_core::{ConnectOptions, SurrealFs};
    use surrealfs_server::chat::ChatConfig;
    use surrealfs_server::embed::Embedder;
    use surrealfs_server::gateway::SurrealFsGateway;
    use surrealfs_server::http::serve_http;

    let mut bind_addr = std::env::var("SURREALFS_BIND").unwrap_or_else(|_| {
        let port = std::env::var("SURREALFS_PORT").unwrap_or_else(|_| "8787".to_string());
        format!("127.0.0.1:{}", port)
    });
    let mut url =
        std::env::var("SURREALDB_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_string());
    let mut ns = std::env::var("SURREALDB_NS").unwrap_or_else(|_| "surrealfs".to_string());
    let mut db = std::env::var("SURREALDB_DB").unwrap_or_else(|_| "main".to_string());
    let mut user = std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".to_string());
    let mut pass = std::env::var("SURREALDB_PASS").unwrap_or_else(|_| "root".to_string());
    let token = std::env::var("SURREALDB_TOKEN").ok();
    let mut static_dir: Option<PathBuf> = std::env::var("SURREALFS_STATIC_DIR")
        .ok()
        .map(PathBuf::from);
    let mut nfs_port: Option<u16> = None;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    bind_addr = v.clone();
                }
            }
            "--port" | "-p" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    bind_addr = format!("127.0.0.1:{}", v);
                }
            }
            "--url" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    url = v.clone();
                }
            }
            "--ns" | "--namespace" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    ns = v.clone();
                }
            }
            "--db" | "--database" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    db = v.clone();
                }
            }
            "--user" | "-u" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    user = v.clone();
                }
            }
            "--pass" | "--password" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    pass = v.clone();
                }
            }
            "--static-dir" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    static_dir = Some(PathBuf::from(v));
                }
            }
            "--nfs-port" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    nfs_port = v.parse().ok();
                }
            }
            _ => {}
        }
        i += 1;
    }

    if static_dir.is_none() {
        let default_static = PathBuf::from("surrealfs/browser/static");
        if default_static.is_dir() {
            static_dir = Some(default_static);
        }
    }

    let (auth_user, auth_pass) = if let Some(tok) = token.filter(|s| !s.trim().is_empty()) {
        (None, Some(tok))
    } else {
        (Some(user), Some(pass))
    };

    let fs = SurrealFs::connect_http(ConnectOptions {
        url: url.clone(),
        ns: ns.clone(),
        db: db.clone(),
        user: auth_user,
        pass: auth_pass,
        caller: Some("root".to_string()),
    })?;
    if let Err(e) = fs.apply_schema().await {
        eprintln!("Warning: apply_schema returned: {}", e);
    }

    if let Some(port) = nfs_port {
        let nfs_fs = fs.clone();
        tokio::spawn(async move {
            if let Err(e) = surrealfs_server::nfs::serve_nfs(nfs_fs, port).await {
                eprintln!("NFSv3 server error: {}", e);
            }
        });
        println!("SurrealFS NFSv3 listening on 127.0.0.1:{}", port);
    }

    let embedder = Embedder::from_env();
    let chat_cfg = ChatConfig::from_env();
    let server_token = std::env::var("SURREALFS_SERVER_TOKEN").ok();

    println!(
        "SurrealFS Server listening on http://{} -> {} ({}/{}) [embeddings={}, chat={}]",
        bind_addr,
        url,
        ns,
        db,
        embedder.is_some(),
        chat_cfg.is_some()
    );

    surrealfs_server::gateway::check_exposure(&bind_addr, server_token.as_deref())
        .map_err(anyhow::Error::msg)?;

    let gateway = SurrealFsGateway::new(fs)
        .with_embedder(embedder)
        .with_chat(chat_cfg)
        .with_server_token(server_token)
        .with_static_dir(static_dir);

    serve_http(&bind_addr, gateway).await?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
