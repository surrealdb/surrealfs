mod common;

use common::create_test_fs;
use std::sync::Arc;
use surrealfs_core::chunking::FastCdcConfig;
use surrealfs_core::{BlobStore, MemoryBlobStore};

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

fn payload(entries: usize) -> Vec<u8> {
    let mut payload = Vec::new();
    for i in 0..entries {
        payload.extend_from_slice(format!("Payload chunk entry index #{:06}\n", i).as_bytes());
    }
    payload
}

async fn blob_rows(fs: &surrealfs_core::SurrealFs) -> Vec<serde_json::Value> {
    let mut res = fs
        .client()
        .query("SELECT record::id(id) AS id, tier, bytes, location FROM blob")
        .await
        .unwrap();
    res.take(0usize).unwrap()
}

#[tokio::test]
async fn test_object_tier_chunks_live_in_the_blob_store() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store.clone());
    let data = payload(50_000);

    fs.upload_file("/o/big.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();

    let rows = blob_rows(&fs).await;
    assert!(rows.len() > 1, "expected several chunks");
    assert_eq!(store.len(), rows.len());
    for row in &rows {
        assert_eq!(row["tier"], "object");
        assert!(
            row["bytes"].is_null(),
            "object chunks keep no bytes in the row"
        );
        let key = row["location"].as_str().unwrap();
        assert!(store.head(key).await.unwrap().is_some());
    }

    assert_eq!(fs.read_bytes("/o/big.bin").await.unwrap(), data);
    let mid = fs.read_range("/o/big.bin", 20_000, 25_000).await.unwrap();
    assert_eq!(mid, &data[20_000..45_000]);
    let tail = fs
        .read_range("/o/big.bin", data.len() as u64 - 10, 1_000)
        .await
        .unwrap();
    assert_eq!(tail, &data[data.len() - 10..]);
}

#[tokio::test]
async fn test_object_chunks_are_deduplicated_across_files() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store.clone());
    let data = payload(20_000);

    fs.upload_file("/d/one.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();
    let stored = store.len();
    fs.upload_file("/d/two.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();

    assert_eq!(store.len(), stored);
    assert_eq!(fs.read_bytes("/d/two.bin").await.unwrap(), data);
}

#[tokio::test]
async fn test_whole_file_read_of_inline_tier_chunks() {
    let fs = create_test_fs().await;
    let data = payload(20_000);

    fs.upload_file("/i/big.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();

    assert_eq!(fs.read_bytes("/i/big.bin").await.unwrap(), data);
}

#[tokio::test]
async fn test_object_chunks_need_a_configured_store() {
    let store = Arc::new(MemoryBlobStore::new());
    let writer = create_test_fs().await.with_blob_store(store);
    writer
        .upload_file(
            "/n/x.bin",
            &payload(5_000),
            None,
            Some(FastCdcConfig::small()),
        )
        .await
        .unwrap();
    let no_store = writer.without_blob_store();

    let err = no_store.read_bytes("/n/x.bin").await.unwrap_err();
    assert!(
        matches!(err, surrealfs_core::SurrealFsError::Storage(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn test_gc_removes_rows_and_unreferenced_objects() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store.clone());
    let data = payload(20_000);

    fs.upload_file("/g/gone.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();
    let keep = payload(3_000);
    fs.upload_file("/g/keep.bin", &keep, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();
    store
        .put("blobs/ab/abcdef", b"stray".to_vec())
        .await
        .unwrap();

    fs.rm("/g/gone.bin", false).await.unwrap();
    let removed = fs.gc_blobs(0).await.unwrap();

    assert!(removed > 0, "rows of the removed file are collected");
    assert!(store.head("blobs/ab/abcdef").await.unwrap().is_none());
    assert_eq!(fs.read_bytes("/g/keep.bin").await.unwrap(), keep);
    let rows = blob_rows(&fs).await;
    assert_eq!(
        store.len(),
        rows.len(),
        "every object has a row, every row an object"
    );
}

fn pieces(data: &[u8], size: usize) -> surrealfs_core::ByteStream {
    let parts: Vec<surrealfs_core::Result<Vec<u8>>> =
        data.chunks(size).map(|c| Ok(c.to_vec())).collect();
    Box::pin(futures::stream::iter(parts))
}

async fn collect(mut body: surrealfs_core::ByteStream) -> Vec<u8> {
    use futures::StreamExt;
    let mut out = Vec::new();
    while let Some(piece) = body.next().await {
        out.extend_from_slice(&piece.unwrap());
    }
    out
}

fn big_payload() -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..120_000u32 {
        data.extend_from_slice(format!("{i} {}\n", i.wrapping_mul(2654435761)).as_bytes());
    }
    assert!(data.len() > surrealfs_core::INLINE_MAX_BYTES);
    data
}

#[tokio::test]
async fn test_streamed_upload_stores_the_same_chunks_as_a_one_shot_upload() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store.clone());
    let data = big_payload();

    fs.upload_file("/s/one-shot.bin", &data, None, Some(FastCdcConfig::small()))
        .await
        .unwrap();
    let stored = store.len();
    let entry = fs
        .upload_stream(
            "/s/streamed.bin",
            pieces(&data, 9_001),
            None,
            Some(FastCdcConfig::small()),
        )
        .await
        .unwrap();

    assert_eq!(entry.size as usize, data.len());
    assert_eq!(store.len(), stored, "identical content adds no objects");
    assert_eq!(fs.read_bytes("/s/streamed.bin").await.unwrap(), data);
}

#[tokio::test]
async fn test_small_streamed_upload_is_inline() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store.clone());

    fs.upload_stream("/s/small.txt", pieces(b"hello stream", 4), None, None)
        .await
        .unwrap();

    assert!(store.is_empty());
    assert_eq!(
        fs.read_bytes("/s/small.txt").await.unwrap(),
        b"hello stream"
    );
    fs.upload_stream("/s/empty.txt", pieces(b"", 4), None, None)
        .await
        .unwrap();
    assert!(fs.read_bytes("/s/empty.txt").await.unwrap().is_empty());
}

#[tokio::test]
async fn test_read_stream_ranges() {
    use surrealfs_core::ByteRange;
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store);
    let data = big_payload();
    let n = data.len() as u64;
    fs.upload_stream(
        "/r/f.bin",
        pieces(&data, 50_000),
        None,
        Some(FastCdcConfig::small()),
    )
    .await
    .unwrap();

    let whole = fs.read_stream("/r/f.bin", ByteRange::Full).await.unwrap();
    assert_eq!(whole.size, n);
    assert_eq!(whole.range, 0..n);
    assert_eq!(collect(whole.body).await, data);

    let cases = [
        (ByteRange::Bounded(0, 0), 0..1),
        (ByteRange::Bounded(100, 70_000), 100..70_001),
        (ByteRange::Bounded(n - 5, n + 1_000), n - 5..n),
        (ByteRange::From(n - 10), n - 10..n),
        (ByteRange::Suffix(1_234), n - 1_234..n),
        (ByteRange::Suffix(n + 99), 0..n),
    ];
    for (range, want) in cases {
        let file = fs.read_stream("/r/f.bin", range).await.unwrap();
        assert_eq!(file.range, want, "{range:?}");
        assert_eq!(
            collect(file.body).await,
            &data[want.start as usize..want.end as usize]
        );
    }

    for bad in [
        ByteRange::From(n),
        ByteRange::Bounded(n, n + 5),
        ByteRange::Suffix(0),
    ] {
        let err = fs.read_stream("/r/f.bin", bad).await.err().unwrap();
        assert!(matches!(
            err,
            surrealfs_core::SurrealFsError::InvalidArguments(_)
        ));
    }
}

#[tokio::test]
async fn test_read_stream_of_an_inline_file_and_a_missing_one() {
    use surrealfs_core::ByteRange;
    let fs = create_test_fs().await;
    fs.write_bytes("/r/small.bin", b"0123456789", None)
        .await
        .unwrap();

    let file = fs
        .read_stream("/r/small.bin", ByteRange::Bounded(2, 4))
        .await
        .unwrap();
    assert_eq!(file.size, 10);
    assert_eq!(collect(file.body).await, b"234");
    assert!(fs
        .read_stream("/r/none.bin", ByteRange::Full)
        .await
        .is_err());
}

#[tokio::test]
async fn test_overwriting_a_chunked_file_with_inline_content_drops_its_chunks() {
    let store = Arc::new(MemoryBlobStore::new());
    let fs = create_test_fs().await.with_blob_store(store);
    fs.upload_stream("/w/f.bin", pieces(&big_payload(), 50_000), None, None)
        .await
        .unwrap();

    fs.write_bytes("/w/f.bin", b"now small", None)
        .await
        .unwrap();

    assert_eq!(fs.read_bytes("/w/f.bin").await.unwrap(), b"now small");
    assert_eq!(fs.read_range("/w/f.bin", 0, 3).await.unwrap(), b"now");
}
