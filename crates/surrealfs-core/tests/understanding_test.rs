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
