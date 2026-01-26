use std::env;
use std::path::PathBuf;

use regex::Regex;
use surrealdb::Surreal;
use surrealdb::engine::any::connect;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::auth::Root;

use surrealfs::SurrealFs;

#[tokio::main]
async fn main() -> surrealfs::Result<()> {
    // Demo using either a file-backed engine (default) or a remote SurrealDB.
    // Set env SURREALFS_REMOTE=1 to use remote at ws://127.0.0.1:8000 with root/root.
    let use_remote = env::var("SURREALFS_REMOTE").is_ok();

    if use_remote {
        println!("Using remote SurrealDB at ws://127.0.0.1:8000 (ns=surrealfs, db=demo)");
        let db = connect("ws://127.0.0.1:8000").await?;
        println!("Signing in as root");
        db.signin(Root {
            username: "root",
            password: "root",
        })
        .await?;
        println!("Using namespace and database");
        db.use_ns("surrealfs").use_db("demo").await?;
        let fs = SurrealFs::new(db);
        run_demo(fs).await
    } else {
        println!("Using RocksDB-backed SurrealDB at ./demo-db (ns=surrealfs, db=demo)");
        let db_path = PathBuf::from("./demo-db");
        let db = Surreal::new::<RocksDb>(db_path.as_path()).await?;
        db.use_ns("surrealfs").use_db("demo").await?;
        let fs = SurrealFs::new(db);
        run_demo(fs).await
    }
}

async fn run_demo<DB>(fs: SurrealFs<DB>) -> surrealfs::Result<()>
where
    DB: surrealdb::Connection,
{
    fs.mkdir_p("/demo").await?;
    fs.write_file("/demo/hello.txt", "hello\nworld\nfrom surrealfs")
        .await?;

    println!("ls /demo:");
    for entry in fs.ls("/demo").await? {
        let kind = if entry.is_dir { "dir" } else { "file" };
        println!("  {} ({})", entry.name, kind);
    }

    println!("\ncat /demo/hello.txt:");
    println!("{}", fs.cat("/demo/hello.txt").await?);

    println!("tail -n 2 /demo/hello.txt:");
    for line in fs.tail("/demo/hello.txt", 2).await? {
        println!("{}", line);
    }

    println!("\ngrep 'world' /demo recursively:");
    let regex = Regex::new("world").unwrap();
    for m in fs.grep(&regex, "/demo", true).await? {
        println!("{}:{}: {}", m.path, m.line_number, m.line);
    }

    println!("\nnl /demo/hello.txt:");
    for line in fs.nl("/demo/hello.txt", 1).await? {
        println!("{:>4}  {}", line.number, line.line);
    }

    Ok(())
}
