mod common;

use common::create_test_fs;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use surrealfs_core::{compute_signature, ArchiveFormat, WebhookDispatcher};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn test_archive_export_and_import_roundtrip() {
    let fs = create_test_fs().await;

    // Populate a test directory structure
    fs.mkdir("/workspace/src", true).await.expect("mkdir src");
    fs.mkdir("/workspace/docs", true).await.expect("mkdir docs");
    fs.write_text(
        "/workspace/src/lib.rs",
        "pub fn answer() -> u32 { 42 }",
        None,
    )
    .await
    .expect("write lib.rs");
    fs.write_text(
        "/workspace/docs/guide.md",
        "# Guide\nWelcome to SurrealFS",
        None,
    )
    .await
    .expect("write guide.md");
    fs.write_bytes("/workspace/binary.dat", &[0xDE, 0xAD, 0xBE, 0xEF], None)
        .await
        .expect("write binary");

    // 1. Tar Format
    let tar_bytes = fs
        .export_tree("/workspace", ArchiveFormat::Tar, None)
        .await
        .expect("export tar");
    assert!(!tar_bytes.is_empty());

    let tar_report = fs
        .import_tree("/imported_tar", &tar_bytes, ArchiveFormat::Tar)
        .await
        .expect("import tar");
    assert_eq!(tar_report.files_imported, 3);
    assert_eq!(
        fs.read_text("/imported_tar/src/lib.rs").await.unwrap(),
        "pub fn answer() -> u32 { 42 }"
    );
    assert_eq!(
        fs.read_text("/imported_tar/docs/guide.md").await.unwrap(),
        "# Guide\nWelcome to SurrealFS"
    );
    assert_eq!(
        fs.read_bytes("/imported_tar/binary.dat").await.unwrap(),
        vec![0xDE, 0xAD, 0xBE, 0xEF]
    );

    // 2. TarGz Format
    let gz_bytes = fs
        .export_tree("/workspace", ArchiveFormat::TarGz, None)
        .await
        .expect("export targz");
    assert!(!gz_bytes.is_empty());

    let gz_report = fs
        .import_tree("/imported_gz", &gz_bytes, ArchiveFormat::TarGz)
        .await
        .expect("import targz");
    assert_eq!(gz_report.files_imported, 3);
    assert_eq!(
        fs.read_text("/imported_gz/src/lib.rs").await.unwrap(),
        "pub fn answer() -> u32 { 42 }"
    );

    // 3. Zip Format
    let zip_bytes = fs
        .export_tree("/workspace", ArchiveFormat::Zip, None)
        .await
        .expect("export zip");
    assert!(!zip_bytes.is_empty());

    let zip_report = fs
        .import_tree("/imported_zip", &zip_bytes, ArchiveFormat::Zip)
        .await
        .expect("import zip");
    assert_eq!(zip_report.files_imported, 3);
    assert_eq!(
        fs.read_text("/imported_zip/docs/guide.md").await.unwrap(),
        "# Guide\nWelcome to SurrealFS"
    );
    assert_eq!(
        fs.read_bytes("/imported_zip/binary.dat").await.unwrap(),
        vec![0xDE, 0xAD, 0xBE, 0xEF]
    );
}

#[tokio::test]
async fn test_webhook_dispatcher_hmac_and_delivery() {
    let fs = create_test_fs().await;

    // Start a mock HTTP receiver
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let port = listener.local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{}/webhook", port);
    let secret = "whsec_test_secret_1234567890";

    let received_count = Arc::new(AtomicUsize::new(0));
    let r_count = received_count.clone();

    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            if n > 0 {
                let req_str = String::from_utf8_lossy(&buf[..n]);
                // Verify signature header exists
                if req_str.contains("x-surrealfs-signature: sha256=")
                    || req_str.contains("X-SurrealFS-Signature: sha256=")
                {
                    r_count.fetch_add(1, Ordering::SeqCst);
                }
                let response =
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        }
    });

    // Register webhook in SurrealFS
    let hook = fs
        .create_webhook("/data/*", &["file_created", "file_updated"], &url, secret)
        .await
        .expect("create_webhook");

    let dispatcher = WebhookDispatcher::new();
    let payload = json!({
        "file": "/data/test.json",
        "action": "create",
        "size": 128
    });

    // Verify HMAC calculation matches
    let body_bytes = serde_json::to_vec(&payload).unwrap();
    let expected_sig = compute_signature(secret, &body_bytes);
    assert!(expected_sig.starts_with("sha256="));

    // Dispatch event
    let deliveries = dispatcher
        .dispatch_event(&fs, "file_created", "/data/test.json", &payload)
        .await
        .expect("dispatch_event");

    assert_eq!(deliveries.len(), 1);
    assert!(deliveries[0].success);
    assert_eq!(deliveries[0].status_code, Some(200));
    assert_eq!(deliveries[0].attempts, 1);
    assert_eq!(deliveries[0].webhook_id, hook.id);

    // Verify receiver received the payload with signature
    assert!(received_count.load(Ordering::SeqCst) >= 1);
}
