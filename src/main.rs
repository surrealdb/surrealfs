use std::env;
use std::path::PathBuf;

use surrealdb::Surreal;
use surrealdb::engine::any::connect;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::auth::Root;

use surrealfs::SurrealFs;

mod commands;
mod repl;

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
        repl::run(fs).await
    } else {
        println!("Using RocksDB-backed SurrealDB at ./demo-db (ns=surrealfs, db=demo)");
        let db_path = PathBuf::from("./demo-db");
        let db = Surreal::new::<RocksDb>(db_path.as_path()).await?;
        db.use_ns("surrealfs").use_db("demo").await?;
        let fs = SurrealFs::new(db);
        repl::run(fs).await
    }
}
