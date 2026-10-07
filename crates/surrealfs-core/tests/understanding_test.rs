mod common;

use common::create_test_fs;

#[tokio::test]
async fn test_symbols_and_definition() {
    let fs = create_test_fs().await;

    let py_code = r#"
class DatabaseEngine:
    """High-performance database connector."""
    def query(self, q: str):
        return []

def initialize_pool(size: int = 10):
    """Initialize worker connection pool."""
    return True
"#;

    fs.write_text("/src/engine.py", py_code, None)
        .await
        .expect("write_text failed");

    // Index symbols
    let sym_count = fs
        .index_file_symbols("/src/engine.py")
        .await
        .expect("index_file_symbols failed");
    assert_eq!(sym_count, 3);

    // List symbols for file
    let symbols = fs.symbols("/src/engine.py").await.expect("symbols failed");
    assert_eq!(symbols.len(), 3);

    // Find symbol definition
    let defs = fs
        .definition("DatabaseEngine")
        .await
        .expect("definition failed");
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].name, "DatabaseEngine");
    assert_eq!(defs[0].kind, "class");
    assert_eq!(
        defs[0].doc.as_deref(),
        Some("High-performance database connector.")
    );

    // Find definition by function name
    let fn_defs = fs
        .definition("initialize_pool")
        .await
        .expect("definition failed");
    assert_eq!(fn_defs.len(), 1);
    assert_eq!(fn_defs[0].name, "initialize_pool");
    assert_eq!(fn_defs[0].kind, "function");
}

#[tokio::test]
async fn test_folder_digest() {
    let fs = create_test_fs().await;

    fs.write_text(
        "/docs/architecture.md",
        "# Architecture\nOverview of the system components and data pipeline.",
        None,
    )
    .await
    .expect("write_text failed");
    fs.write_text(
        "/docs/security.md",
        "# Security Policy\nRules and authentication flow.",
        None,
    )
    .await
    .expect("write_text failed");

    let digest = fs.digest("/docs").await.expect("digest failed");
    assert_eq!(digest.path, "/docs");
    assert_eq!(digest.total_readable, 2);
    assert_eq!(digest.notable_files.len(), 2);
    assert!(digest.summary.contains("Folder /docs"));
}

#[tokio::test]
async fn test_context_pack() {
    let fs = create_test_fs().await;

    fs.write_text(
        "/src/auth.py",
        "def authenticate(token: str):\n    return token == 'secret'\n",
        None,
    )
    .await
    .expect("write_text failed");

    fs.write_text(
        "/src/server.py",
        "def run_server():\n    print('Starting API server on port 8000')\n",
        None,
    )
    .await
    .expect("write_text failed");

    let packed = fs
        .pack("How does authentication work?", 1000, Some("/src"))
        .await
        .expect("pack failed");

    assert_eq!(packed.question, "How does authentication work?");
    assert!(packed.used_tokens <= 1000);
}

#[tokio::test]
async fn test_pipeline_job_claiming_and_completion() {
    let fs = create_test_fs().await;

    fs.write_text(
        "/pipeline/data.txt",
        "Some new data for understanding",
        None,
    )
    .await
    .expect("write_text failed");

    // Claim jobs enqueued by evt_file_jobs
    let jobs = fs
        .claim_jobs("worker-test-1", &["detect", "chunk", "symbols"], 10)
        .await
        .expect("claim_jobs failed");

    assert!(!jobs.is_empty());
    let job_id = &jobs[0].id;

    // Complete the first job
    fs.complete_job(job_id, None)
        .await
        .expect("complete_job failed");
}

#[tokio::test]
async fn test_entities_and_mentions() {
    let fs = create_test_fs().await;

    fs.write_text(
        "/sec/review.md",
        "Configured AWS and Okta SSO with ticket SEC-999 #security",
        None,
    )
    .await
    .expect("write_text failed");

    let entities = vec![
        surrealfs_core::understanding::ExtractedEntity {
            name: "Okta".to_string(),
            kind: "system".to_string(),
        },
        surrealfs_core::understanding::ExtractedEntity {
            name: "SEC-999".to_string(),
            kind: "ticket".to_string(),
        },
    ];

    fs.record_entities("/sec/review.md", &entities)
        .await
        .expect("record_entities failed");

    let retrieved = fs
        .entities("/sec/review.md")
        .await
        .expect("entities failed");
    assert_eq!(retrieved.len(), 2);
    let names: Vec<String> = retrieved.into_iter().map(|e| e.name).collect();
    assert!(names.contains(&"Okta".to_string()));
    assert!(names.contains(&"SEC-999".to_string()));
}

#[tokio::test]
async fn test_tabular_data_load_and_query() {
    let fs = create_test_fs().await;

    let csv_content = "user_id,username,active,balance\n101,alice,true,500.50\n102,bob,false,0.0\n";
    fs.write_text("/data/accounts.csv", csv_content, None)
        .await
        .expect("write_text failed");

    let loaded = fs
        .load_tabular_file("/data/accounts.csv", 100)
        .await
        .expect("load_tabular_file failed");
    assert_eq!(loaded, 2);

    let rows = fs
        .query_table("/data/accounts.csv", Some(10))
        .await
        .expect("query_table failed");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].data["username"], "alice");
    assert_eq!(rows[0].data["active"], true);
    assert_eq!(rows[1].data["username"], "bob");
    assert_eq!(rows[1].data["active"], false);
}

#[tokio::test]
async fn test_simhash_near_duplicates_check() {
    let fs = create_test_fs().await;

    let doc1 = "SurrealFS provides unified storage with ACID transactions and vector embeddings for AI agents.";
    let doc2 = "SurrealFS provides unified storage with ACID transactions and vector embeddings for autonomous agents.";

    fs.write_text("/docs/doc1.txt", doc1, None)
        .await
        .expect("write_text failed");

    let h1 = surrealfs_core::understanding::compute_simhash(doc1);
    fs.set_simhash("/docs/doc1.txt", h1)
        .await
        .expect("set_simhash failed");

    let duplicates = fs
        .check_near_duplicates(doc2, 0.80)
        .await
        .expect("check_near_duplicates failed");
    assert!(!duplicates.is_empty());
    assert_eq!(duplicates[0].0, "/docs/doc1.txt");
    assert!(duplicates[0].1 >= 0.80);
}

#[tokio::test]
async fn test_git_repository_import() {
    let fs = create_test_fs().await;

    // Create a temporary directory structure representing a git repository
    let temp_dir = std::env::temp_dir().join(format!("sfs-git-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("src")).unwrap();

    std::fs::write(temp_dir.join(".gitignore"), "*.log\nbuild/\nsecret.env\n").unwrap();
    std::fs::write(temp_dir.join("README.md"), "# Project X\nDocumentation.").unwrap();
    std::fs::write(
        temp_dir.join("src/main.rs"),
        "fn main() { println!(\"Hi\"); }",
    )
    .unwrap();
    std::fs::write(temp_dir.join("debug.log"), "IGNORE ME").unwrap();
    std::fs::write(temp_dir.join("secret.env"), "KEY=123").unwrap();

    let opts = surrealfs_core::GitImportOptions {
        repo_path: temp_dir.to_string_lossy().to_string(),
        target_path: "/projects/imported".to_string(),
        max_commits: Some(10),
        branch: None,
    };

    let result = fs.import_git(&opts).await.expect("import_git failed");
    assert_eq!(result.imported_files, 3); // .gitignore, README.md, and src/main.rs (debug.log and secret.env ignored)
    assert_eq!(result.target_path, "/projects/imported");

    let readme = fs.read_text("/projects/imported/README.md").await.unwrap();
    assert!(readme.contains("Project X"));

    let main_rs = fs
        .read_text("/projects/imported/src/main.rs")
        .await
        .unwrap();
    assert!(main_rs.contains("println!"));

    // Ignored files should not exist
    assert!(fs.read_text("/projects/imported/debug.log").await.is_err());
    assert!(fs.read_text("/projects/imported/secret.env").await.is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
