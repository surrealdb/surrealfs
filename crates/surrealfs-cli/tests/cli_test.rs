mod common;

use common::{create_test_fs, get_server_url};
use surrealfs_cli::{
    run_cli, Cli, Commands, CrdtCommands, CredentialCommands, GroupCommands, LockCommands, LsArgs,
    ShareCommands, WebhookCommands,
};

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

    // Mount
    let tmp_mount = format!("/tmp/surrealfs_test_mount_{}", common::rand_id());
    let mount_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Mount {
            mountpoint: tmp_mount.clone(),
            branch: "dev".to_string(),
            nfs: false,
        },
    ))
    .await
    .unwrap();
    assert!(mount_out.contains("Mounting SurrealFS"));
    assert!(mount_out.contains(".surrealfs"));
    let _ = std::fs::remove_dir_all(&tmp_mount);
}

#[tokio::test]
async fn test_cli_crdt_and_chunk() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;

    // Write a markdown document
    let md = "# Title\n\n## Section 1\nContent 1\n\n## Section 2\nContent 2\n";
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/docs/spec.md".to_string(),
            content: md.to_string(),
        },
    ))
    .await
    .unwrap();

    // Chunk command
    let chunk_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Chunk {
            path: "/docs/spec.md".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(chunk_out.contains("Title > Section 1"));
    assert!(chunk_out.contains("Title > Section 2"));

    // Enable CRDT
    let crdt_enable = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Crdt {
            action: CrdtCommands::Enable {
                path: "/docs/spec.md".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(crdt_enable.contains("Enabled CRDT on /docs/spec.md"));

    // Write update under CRDT
    let updated_md = "# Title\n\n## Section 1\nUpdated Content 1\n\n## Section 2\nContent 2\n";
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/docs/spec.md".to_string(),
            content: updated_md.to_string(),
        },
    ))
    .await
    .unwrap();

    // Verify cat reads back new CRDT content
    let cat_res = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Cat {
            path: "/docs/spec.md".to_string(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(cat_res, updated_md);

    // Compact CRDT
    let crdt_compact = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Crdt {
            action: CrdtCommands::Compact {
                path: "/docs/spec.md".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(crdt_compact.contains("Compacted CRDT on /docs/spec.md"));
}

#[tokio::test]
async fn test_cli_understanding_pipeline() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;

    let py_code = "def authenticate_user(token: str):\n    \"\"\"Authenticate user from auth header.\"\"\"\n    return token == 'valid'\n";

    // 1. Write file
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/src/auth.py".to_string(),
            content: py_code.to_string(),
        },
    ))
    .await
    .unwrap();

    // 2. Detect
    let detect_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Detect {
            path: "/src/auth.py".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(detect_out.contains("python"));
    assert!(detect_out.contains("text/x-python"));

    // 3. Symbols index
    let sym_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Symbols {
            path: "/src/auth.py".to_string(),
            index: true,
        },
    ))
    .await
    .unwrap();
    assert!(sym_out.contains("authenticate_user"));

    // 4. Definition
    let def_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Definition {
            name: "authenticate_user".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(def_out.contains("authenticate_user"));
    assert!(def_out.contains("function"));

    // 5. Digest
    let dig_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Digest {
            path: "/src".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(dig_out.contains("/src"));

    // 6. Pack
    let pack_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Pack {
            question: "How is a user authenticated?".to_string(),
            budget: 2000,
            scope: Some("/src".to_string()),
        },
    ))
    .await
    .unwrap();
    assert!(pack_out.contains("Context for: How is a user authenticated?"));

    // 7. Jobs claim and complete
    let claim_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Jobs {
            action: surrealfs_cli::JobCommands::Claim {
                worker: "test-cli-worker".to_string(),
                limit: 5,
            },
        },
    ))
    .await
    .unwrap();
    assert!(claim_out.contains("detect") || claim_out.contains("symbols"));

    // 8. Tabular load and query
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/data/items.csv".to_string(),
            content: "id,name,qty\n10,Widget,100\n20,Gadget,50\n".to_string(),
        },
    ))
    .await
    .unwrap();

    let load_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::TableLoad {
            path: "/data/items.csv".to_string(),
            max_rows: 100,
        },
    ))
    .await
    .unwrap();
    assert!(load_out.contains("Loaded 2 rows"));

    let query_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::TableQuery {
            path: "/data/items.csv".to_string(),
            limit: 10,
        },
    ))
    .await
    .unwrap();
    assert!(query_out.contains("Widget"));
    assert!(query_out.contains("Gadget"));

    // 9. Near duplicates check
    let near_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::NearDups {
            path_or_text: "def authenticate_user(username, password): return username == 'admin'"
                .to_string(),
            threshold: 0.50,
        },
    ))
    .await
    .unwrap();
    assert!(near_out.starts_with('['));

    // 10. Git import
    let temp_repo = std::env::temp_dir().join(format!("sfs-cli-git-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_repo);
    std::fs::create_dir_all(&temp_repo).unwrap();
    std::fs::write(temp_repo.join("hello.txt"), "Git imported file!").unwrap();

    let git_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::ImportGit {
            repo_path: temp_repo.to_string_lossy().to_string(),
            target_path: "/git/imported".to_string(),
            max_commits: Some(5),
        },
    ))
    .await
    .unwrap();
    assert!(git_out.contains("Imported 1 files"));

    let _ = std::fs::remove_dir_all(&temp_repo);
}

#[tokio::test]
async fn test_cli_credentials_and_serve_dispatch() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = common::get_server_url().await;

    // 1. Create credential via CLI
    let create_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Credential {
            action: CredentialCommands::Create {
                kind: "webdav".to_string(),
                identifier: "alice".to_string(),
                secret: "pass123".to_string(),
                user_id: "user:alice".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(create_out.contains("Created credential alice (webdav) for user:alice"));

    // 2. List credentials
    let list_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Credential {
            action: CredentialCommands::List { user_id: None },
        },
    ))
    .await
    .unwrap();
    assert!(list_out.contains("alice"));
    assert!(list_out.contains("user:alice"));

    // 3. Revoke credential
    let revoke_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Credential {
            action: CredentialCommands::Revoke {
                kind: "webdav".to_string(),
                identifier: "alice".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(revoke_out.contains("Revoked credential alice (webdav)"));

    // 4. Test Mount --nfs command builder
    let nfs_mount_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Mount {
            mountpoint: "/mnt/surrealfs".to_string(),
            branch: "main".to_string(),
            nfs: true,
        },
    ))
    .await
    .unwrap();
    assert!(nfs_mount_out.contains("To complete NFS mount, run:"));
    assert!(nfs_mount_out.contains("mount -t nfs"));
}

#[tokio::test]
async fn test_cli_phase9_trust_and_operations() {
    let (_fs, ns, db) = create_test_fs().await;
    let url = get_server_url().await;
    let tmp = tempfile::tempdir().unwrap();

    // 1. Group commands
    let g_create = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Group {
            action: GroupCommands::Create {
                name: "devs".to_string(),
                description: Some("Developers".to_string()),
            },
        },
    ))
    .await
    .unwrap();
    assert!(g_create.contains("Group 'devs' created"));

    let g_add = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Group {
            action: GroupCommands::Add {
                group: "devs".to_string(),
                user: "bob".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(g_add.contains("Added 'bob' to group 'devs'"));

    let g_members = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Group {
            action: GroupCommands::Members {
                group: "devs".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(g_members.contains("bob"));

    // 2. Write file, chmod, chgrp, legal hold
    run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Write {
            path: "/audit/ledger.txt".to_string(),
            content: "SEC Financial Records 2026".to_string(),
        },
    ))
    .await
    .unwrap();

    let chgrp_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Chgrp {
            path: "/audit/ledger.txt".to_string(),
            group: Some("devs".to_string()),
        },
    ))
    .await
    .unwrap();
    assert!(chgrp_out.contains("Updated group for /audit/ledger.txt to Some(\"devs\")"));

    let chmod_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Chmod {
            path: "/audit/ledger.txt".to_string(),
            mode: "0640".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(chmod_out.contains("Updated mode for /audit/ledger.txt to 640"));

    let hold_set = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::LegalHold {
            path: "/audit/ledger.txt".to_string(),
            active: Some(true),
        },
    ))
    .await
    .unwrap();
    assert!(hold_set.contains("Set legal hold on /audit/ledger.txt to true"));

    let hold_check = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::LegalHold {
            path: "/audit/ledger.txt".to_string(),
            active: None,
        },
    ))
    .await
    .unwrap();
    assert!(hold_check.contains("Legal hold on /audit/ledger.txt: true"));

    // Deletion should fail due to legal hold
    let rm_fail = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Rm {
            path: "/audit/ledger.txt".to_string(),
            recursive: false,
        },
    ))
    .await;
    assert!(rm_fail.is_err());

    // 3. Share link lifecycle
    let share_create = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Share {
            action: ShareCommands::Create {
                path: "/audit/ledger.txt".to_string(),
                mode: "r".to_string(),
                expires_in_secs: Some(3600),
            },
        },
    ))
    .await
    .unwrap();
    assert!(share_create.contains("Created share link:"));

    let token = share_create
        .lines()
        .find(|l| l.contains("Token:"))
        .map(|l| l.split(':').nth(1).unwrap().trim())
        .unwrap();

    let share_resolve = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Share {
            action: ShareCommands::Resolve {
                token: token.to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(share_resolve.contains("Resolved share link:"));

    let share_revoke = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Share {
            action: ShareCommands::Revoke {
                token: token.to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(share_revoke.contains("Revoked share link"));

    // 4. Webhook registration
    let hook_create = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Webhook {
            action: WebhookCommands::Create {
                pattern: "/audit/*".to_string(),
                events: "create,write".to_string(),
                url: "https://example.com/webhook".to_string(),
                secret: "s3cr3t".to_string(),
            },
        },
    ))
    .await
    .unwrap();
    assert!(hook_create.contains("Created webhook"));

    let hook_list = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Webhook {
            action: WebhookCommands::List,
        },
    ))
    .await
    .unwrap();
    assert!(hook_list.contains("/audit/*"));

    // 5. Export / Import archive
    let export_tar = tmp.path().join("audit.tar.gz");
    let export_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Export {
            path: "/audit".to_string(),
            output_file: export_tar.to_str().unwrap().to_string(),
            format: "targz".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(export_out.contains("Exported /audit"));
    assert!(export_tar.exists());

    let import_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Import {
            archive_file: export_tar.to_str().unwrap().to_string(),
            target_path: "/audit_restored".to_string(),
            format: "targz".to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(import_out.contains("Imported"));

    // 6. Backup & RestoreBackup
    let backup_dir = tmp.path().join("backup_dir");
    let backup_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Backup {
            path: "/audit".to_string(),
            backup_dir: backup_dir.to_str().unwrap().to_string(),
        },
    ))
    .await
    .unwrap();
    assert!(backup_out.contains("Backup complete:"));

    let restore_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::RestoreBackup {
            backup_dir: backup_dir.to_str().unwrap().to_string(),
            target_prefix: "/audit_pitr".to_string(),
            as_of: None,
        },
    ))
    .await
    .unwrap();
    assert!(restore_out.contains("Restore complete:"));

    // 7. Stats
    let stats_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Stats { prometheus: false },
    ))
    .await
    .unwrap();
    assert!(stats_out.contains("\"total_files\""));

    let prom_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Stats { prometheus: true },
    ))
    .await
    .unwrap();
    assert!(prom_out.contains("surrealfs_files_total"));

    // 8. Encrypt & Decrypt
    let enc_input = tmp.path().join("plain.txt");
    std::fs::write(&enc_input, b"Super confidential payload 2026").unwrap();
    let key_hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

    let enc_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Encrypt {
            path: "/secure/data.enc".to_string(),
            input_file: enc_input.to_str().unwrap().to_string(),
            key_hex: key_hex.to_string(),
            key_id: Some("key-01".to_string()),
        },
    ))
    .await
    .unwrap();
    assert!(enc_out.contains("Wrote encrypted file /secure/data.enc"));

    let dec_out = run_cli(make_cli(
        url,
        &ns,
        &db,
        Commands::Decrypt {
            path: "/secure/data.enc".to_string(),
            key_hex: key_hex.to_string(),
            output_file: None,
        },
    ))
    .await
    .unwrap();
    assert!(dec_out.contains("Super confidential payload 2026"));
}
