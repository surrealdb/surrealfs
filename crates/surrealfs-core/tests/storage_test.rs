mod common;

use common::create_test_fs;
use surrealfs_core::chunking::FastCdcConfig;

#[tokio::test]
async fn test_upload_file_chunked_and_read_range() {
    let fs = create_test_fs().await;

    // Generate non-trivial payload with repeating patterns
    let mut payload = Vec::new();
    for i in 0..50_000 {
        payload.extend_from_slice(format!("Payload chunk entry index #{:06}\n", i).as_bytes());
    }
    let total_len = payload.len();

    // Use small FastCDC config to ensure multiple chunks are formed
    let entry = fs
        .upload_file(
            "/datasets/stream.bin",
            &payload,
            None,
            Some(FastCdcConfig::small()),
        )
        .await
        .expect("upload_file failed");

    assert_eq!(entry.path, "/datasets/stream.bin");
    assert_eq!(entry.size as usize, total_len);

    // 1. Read prefix
    let head = fs
        .read_range("/datasets/stream.bin", 0, 1024)
        .await
        .expect("read_range head failed");
    assert_eq!(head, &payload[0..1024]);

    // 2. Read across chunk boundary in the middle
    let mid = fs
        .read_range("/datasets/stream.bin", 20_000, 25_000)
        .await
        .expect("read_range mid failed");
    assert_eq!(mid, &payload[20_000..45_000]);

    // 3. Read suffix
    let tail = fs
        .read_range("/datasets/stream.bin", (total_len - 1500) as u64, 1500)
        .await
        .expect("read_range tail failed");
    assert_eq!(tail, &payload[total_len - 1500..]);

    // 4. Read full payload
    let full = fs
        .read_range("/datasets/stream.bin", 0, total_len as u64)
        .await
        .expect("read_range full failed");
    assert_eq!(full, payload);
}

#[tokio::test]
async fn test_scoped_deduplication() {
    let fs = create_test_fs().await;

    let mut payload = Vec::new();
    for i in 0..20_000 {
        payload.extend_from_slice(format!("Duplicated data row {}\n", i % 100).as_bytes());
    }

    // Upload first file
    let entry1 = fs
        .upload_file(
            "/media/file1.dat",
            &payload,
            None,
            Some(FastCdcConfig::small()),
        )
        .await
        .expect("upload 1 failed");
    assert_eq!(entry1.size as usize, payload.len());

    // Upload identical payload under a second path
    let entry2 = fs
        .upload_file(
            "/media/file2.dat",
            &payload,
            None,
            Some(FastCdcConfig::small()),
        )
        .await
        .expect("upload 2 failed");
    assert_eq!(entry2.size as usize, payload.len());

    // Verify both files can be read back identically
    let read1 = fs
        .read_range("/media/file1.dat", 0, 5000)
        .await
        .expect("read 1 failed");
    let read2 = fs
        .read_range("/media/file2.dat", 0, 5000)
        .await
        .expect("read 2 failed");
    assert_eq!(read1, read2);
    assert_eq!(read1, &payload[0..5000]);
}

#[tokio::test]
async fn test_instant_du_and_gc() {
    let fs = create_test_fs().await;

    let data_a = b"AAAABBBBCCCCDDDD1111222233334444";
    let data_b = b"EEEEFFFFGGGGHHHH555566667777888899990000";

    fs.upload_file(
        "/workspace/a.bin",
        data_a,
        None,
        Some(FastCdcConfig::small()),
    )
    .await
    .unwrap();

    fs.upload_file(
        "/workspace/sub/b.bin",
        data_b,
        None,
        Some(FastCdcConfig::small()),
    )
    .await
    .unwrap();

    let usage = fs.du("/workspace").await.expect("du failed");
    assert_eq!(usage.files, 2);
    assert_eq!(usage.logical_bytes, (data_a.len() + data_b.len()) as u64);

    // Test gc_blobs
    let gc_count = fs.gc_blobs(0).await.expect("gc_blobs failed");
    // Initially all active blobs have refs > 0 so 0 unreferenced blobs are pruned
    assert_eq!(gc_count, 0);
}
