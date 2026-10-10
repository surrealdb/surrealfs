mod common;

use common::create_test_fs;
use surrealfs_core::{
    create_backup, restore_backup, ConnectorManager, GitHubConnector, GoogleDriveConnector,
    SourceDocument,
};
use tempfile::tempdir;

#[tokio::test]
async fn test_e2e_encryption_envelope() {
    let fs = create_test_fs().await;
    let key = [7u8; 32];
    let secret_data = b"TOP SECRET MISSION DIRECTIVE 2026";

    // Write encrypted file
    fs.write_encrypted(
        "/confidential/mission.txt",
        secret_data,
        &key,
        Some("key-001"),
    )
    .await
    .expect("write_encrypted");

    // Stat the file: verify meta.encrypted = true
    let entry = fs
        .stat("/confidential/mission.txt")
        .await
        .expect("stat")
        .expect("file exists");
    let meta = entry.meta.expect("meta");
    assert_eq!(meta.get("encrypted").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        meta.get("encryption_algo").and_then(|v| v.as_str()),
        Some("aes-256-gcm")
    );

    // Read directly via read_bytes (raw ciphertext - should NOT match plaintext)
    let raw = fs
        .read_bytes("/confidential/mission.txt")
        .await
        .expect("read_bytes");
    assert_ne!(raw, secret_data);

    // Read and decrypt with correct key
    let decrypted = fs
        .read_encrypted("/confidential/mission.txt", &key)
        .await
        .expect("read_encrypted");
    assert_eq!(decrypted, secret_data);

    // Read with invalid key fails
    let wrong_key = [99u8; 32];
    assert!(fs
        .read_encrypted("/confidential/mission.txt", &wrong_key)
        .await
        .is_err());
}

#[tokio::test]
async fn test_connector_mirror_sync() {
    let fs = create_test_fs().await;

    // Create mock docs for Google Drive and GitHub
    let gdoc = SourceDocument {
        id: "doc-123".to_string(),
        path: "Engineering/Architecture.md".to_string(),
        title: "Architecture".to_string(),
        content: "# System Architecture\nDistributed SurrealFS".to_string(),
        content_type: "text/markdown".to_string(),
        source_url: "https://docs.google.com/document/d/doc-123".to_string(),
        source_version: Some("v1.4".to_string()),
        owner: Some("alice".to_string()),
        group: None,
        mode: Some(0o644),
    };

    let gh_issue = SourceDocument {
        id: "issue-456".to_string(),
        path: "issues/456.md".to_string(),
        title: "Bug in cache".to_string(),
        content: "---\nid: 456\nstate: open\n---\nFix chunk cache race condition".to_string(),
        content_type: "text/markdown".to_string(),
        source_url: "https://github.com/org/repo/issues/456".to_string(),
        source_version: Some("rev2".to_string()),
        owner: None,
        group: None,
        mode: None,
    };

    let mut mgr = ConnectorManager::new();
    mgr.register(GoogleDriveConnector::new(vec![gdoc]));
    mgr.register(GitHubConnector::new(vec![gh_issue]));

    let reports = mgr.sync_all(&fs).await.expect("sync_all");
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].files_synced, 1);
    assert_eq!(reports[1].files_synced, 1);

    // Verify synced files in SurrealFS under /sources/<connector>/...
    let gdrive_text = fs
        .read_text("/sources/gdrive/Engineering/Architecture.md")
        .await
        .expect("read gdrive mirror");
    assert!(gdrive_text.contains("Distributed SurrealFS"));

    let gh_text = fs
        .read_text("/sources/github/issues/456.md")
        .await
        .expect("read github mirror");
    assert!(gh_text.contains("race condition"));

    // Verify metadata
    let gstat = fs
        .stat("/sources/gdrive/Engineering/Architecture.md")
        .await
        .expect("stat")
        .unwrap();
    let gmeta = gstat.meta.unwrap();
    assert_eq!(gmeta.get("source").and_then(|v| v.as_str()), Some("gdrive"));
    assert_eq!(
        gmeta.get("source_id").and_then(|v| v.as_str()),
        Some("doc-123")
    );
    assert_eq!(
        gmeta.get("source_url").and_then(|v| v.as_str()),
        Some("https://docs.google.com/document/d/doc-123")
    );
}

#[tokio::test]
async fn test_backup_and_restore_cycle() {
    let fs = create_test_fs().await;
    let tmp = tempdir().expect("tempdir");
    let backup_path = tmp.path().join("sfs_backup");

    // Populate data
    fs.mkdir("/dataset/2026", true).await.expect("mkdir");
    fs.write_text(
        "/dataset/2026/report.csv",
        "id,metric,value\n1,m1,100",
        None,
    )
    .await
    .expect("write csv");
    fs.write_bytes("/dataset/2026/weights.bin", &[1, 2, 3, 4, 5, 6, 7, 8], None)
        .await
        .expect("write bin");

    // Create backup
    let manifest = create_backup(&fs, "/dataset", &backup_path)
        .await
        .expect("create_backup");
    assert_eq!(manifest.files.len(), 3); // 1 folder + 2 files
    assert!(backup_path.join("manifest.json").exists());
    assert!(backup_path.join("blobs").exists());

    // Restore backup to /dataset_restored
    let restore_rep = restore_backup(&fs, &backup_path, "/dataset_restored", None)
        .await
        .expect("restore_backup");
    assert_eq!(restore_rep.files_restored, 2);
    assert_eq!(restore_rep.dirs_restored, 1);

    // Verify restored content
    assert_eq!(
        fs.read_text("/dataset_restored/2026/report.csv")
            .await
            .unwrap(),
        "id,metric,value\n1,m1,100"
    );
    assert_eq!(
        fs.read_bytes("/dataset_restored/2026/weights.bin")
            .await
            .unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[tokio::test]
async fn test_observability_and_virtual_stats() {
    let fs = create_test_fs().await;

    fs.mkdir("/obs_test", true).await.expect("mkdir");
    fs.write_text("/obs_test/a.txt", "Alpha content", None)
        .await
        .expect("write a");
    fs.write_text("/obs_test/b.txt", "Beta content", None)
        .await
        .expect("write b");

    // Check stats
    let stats = fs.get_stats().await.expect("get_stats");
    assert!(stats.total_files >= 2);
    assert!(stats.total_directories >= 1);
    assert!(stats.total_bytes >= 20);

    // Read virtual stats file /.surrealfs/stats
    let json_stats = fs
        .read_virtual_stats("/.surrealfs/stats")
        .await
        .expect("read stats");
    assert!(json_stats.contains("\"total_files\""));
    assert!(json_stats.contains("\"total_bytes\""));

    // Read virtual metrics file /.surrealfs/metrics
    let prometheus = fs
        .read_virtual_stats("/.surrealfs/metrics")
        .await
        .expect("read metrics");
    assert!(prometheus.contains("surrealfs_files_total"));
    assert!(prometheus.contains("surrealfs_bytes_total"));
}
