mod common;

use common::{create_test_fs, get_server_url};
use surrealfs_cli::{run_cli, Cli, Commands, LockCommands, LsArgs};

fn make_cli(url: &str, ns: &str, db: &str, cmd: Commands) -> Cli {
    Cli {
        url: url.to_string(),
        ns: ns.to_string(),
        db: db.to_string(),
        user: Some("root".to_string()),
        pass: Some("root".to_string()),
        caller: Some("root".to_string()),
        command: cmd,
    }
}

#[tokio::test]
async fn test_cli_write_cat_ls_rm() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;

    // Status
    let out = run_cli(make_cli(url, &ns, &db, Commands::Status))
        .await
        .unwrap();
    assert!(out.contains("SurrealFS Status: OK"));

    // Write
    let out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/cli_test.txt".to_string(),
            content: "CLI content payload".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(out.contains("Wrote 19 bytes to /cli_test.txt"));

    // Cat
    let out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Cat {
            path: "/cli_test.txt".to_string(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(out, "CLI content payload");

    // Ls
    let out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Ls(LsArgs {
            path: "/".to_string(),
            recursive: false,
        }),
    ))
    .await
    .unwrap();
    assert!(out.contains("/cli_test.txt"));

    // Rm
    let out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Rm {
            path: "/cli_test.txt".to_string(),
            recursive: false,
        },
    ))
    .await
    .unwrap();
    assert!(out.contains("Removed /cli_test.txt"));
}

#[tokio::test]
async fn test_cli_mkdir_cp_mv() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;

    // Mkdir
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Mkdir {
            path: "/sub/dir".to_string(),
            parents: true,
        },
    ))
    .await
    .unwrap();

    // Write file
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/sub/dir/a.txt".to_string(),
            content: "alpha".to_string(),
        },
    ))
    .await
    .unwrap();

    // Cp
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Cp {
            src: "/sub/dir/a.txt".to_string(),
            dst: "/sub/dir/b.txt".to_string(),
            recursive: false,
        },
    ))
    .await
    .unwrap();

    let cat_b = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Cat {
            path: "/sub/dir/b.txt".to_string(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(cat_b, "alpha");

    // Mv
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Mv {
            src: "/sub/dir/b.txt".to_string(),
            dst: "/sub/dir/c.txt".to_string(),
        },
    ))
    .await
    .unwrap();

    let cat_c = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Cat {
            path: "/sub/dir/c.txt".to_string(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(cat_c, "alpha");
}

#[tokio::test]
async fn test_cli_grep_and_lock() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;

    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/src/main.rs".to_string(),
            content: "fn main() {\n    println!(\"MATCH_ME\");\n}\n".to_string(),
        },
    ))
    .await
    .unwrap();

    // Grep
    let grep_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Grep {
            pattern: "MATCH_ME".to_string(),
            path: None,
            recursive: false,
        },
    ))
    .await
    .unwrap();
    assert!(grep_out.contains("/src/main.rs:2: println!(\"MATCH_ME\");"));

    // Lock Acquire
    let lock_acq = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Lock {
            action: LockCommands::Acquire {
                path: "/src/main.rs".to_string(),
                ttl: 60,
                reason: "unit_test".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(lock_acq.contains("Acquired lock on /src/main.rs"));

    // Lock List
    let lock_list = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Lock {
            action: LockCommands::List,
        },
    ))
    .await
    .unwrap();
    assert!(lock_list.contains("/src/main.rs"));

    // Lock Release
    let lock_rel = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Lock {
            action: LockCommands::Release {
                path: "/src/main.rs".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(lock_rel.contains("Released lock on /src/main.rs"));
}
