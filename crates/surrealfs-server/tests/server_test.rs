use std::collections::HashMap;
use surrealfs_core::fs::ConnectOptions;
use surrealfs_core::fs::SurrealFs;
use surrealfs_server::http::{HttpHandler, HttpRequest};
use surrealfs_server::nfs::{NfsFileHandle, NfsServer, NFS3ERR_STALE};
use surrealfs_server::s3::S3Server;
use surrealfs_server::sftp::*;
use surrealfs_server::webdav::WebDavServer;
use surrealfs_server::Authenticator;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

async fn setup_test_fs() -> SurrealFs {
    let url = std::env::var("SURREALFS_TEST_URL")
        .unwrap_or_else(|_| "ws://localhost:8000".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let ns = format!("test_{:x}_{}", nanos, count);
    let opts = ConnectOptions {
        url,
        user: Some("root".into()),
        pass: Some("root".into()),
        ns,
        db: "test".into(),
        caller: Some("test_user".into()),
    };
    let fs = SurrealFs::connect(opts)
        .await
        .expect("Failed to connect to test SurrealDB");

    let schema_file = include_str!("../../../surrealfs/schema/file.surql");
    let schema_auth = include_str!("../../../surrealfs/schema/record_auth.surql");

    fs.client()
        .query(schema_file)
        .await
        .expect("Failed to apply file.surql schema");
    fs.client()
        .query(schema_auth)
        .await
        .expect("Failed to apply record_auth.surql schema");

    fs
}

#[tokio::test]
async fn test_webdav_options_and_crud() {
    let fs = setup_test_fs().await;
    let server = WebDavServer::new(fs.clone(), "/");

    // 1. OPTIONS
    let req = HttpRequest {
        method: "OPTIONS".into(),
        path: "/".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.headers.get("dav").map(|s| s.as_str()), Some("1, 2"));

    // 2. MKCOL /docs
    let req = HttpRequest {
        method: "MKCOL".into(),
        path: "/docs".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 201);

    // 3. PUT /docs/hello.txt
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: b"Hello WebDAV World".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 201);
    let etag = res.headers.get("etag").cloned().unwrap();
    assert!(etag.starts_with("\"gen-"));

    // 4. PROPFIND /docs
    let mut headers = HashMap::new();
    headers.insert("depth".into(), "1".into());
    let req = HttpRequest {
        method: "PROPFIND".into(),
        path: "/docs".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers,
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 207); // Multi-Status
    let body_str = String::from_utf8(res.body).unwrap();
    assert!(body_str.contains("<D:multistatus"));
    assert!(body_str.contains("hello.txt"));

    // 5. GET with Range header (§19.4, §22.1)
    let mut headers = HashMap::new();
    headers.insert("range".into(), "bytes=0-4".into());
    let req = HttpRequest {
        method: "GET".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers,
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 206);
    assert_eq!(res.body, b"Hello");

    // 6. Concurrency control: PUT with If-Match
    let mut headers = HashMap::new();
    headers.insert("if-match".into(), "\"gen-9999\"".into());
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers,
        body: b"Stale update".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 412); // Precondition Failed

    // 7. LOCK & UNLOCK (§22.1)
    let req = HttpRequest {
        method: "LOCK".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    assert!(res.headers.contains_key("lock-token"));

    let req = HttpRequest {
        method: "UNLOCK".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 204);

    // 8. AppleDouble absorption (§22.1)
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/docs/._hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: b"metadata".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 201); // Absorbed
}

#[tokio::test]
async fn test_s3_rest_api_and_multipart() {
    let fs = setup_test_fs().await;
    let server = S3Server::new(fs.clone());

    // 1. CreateBucket PUT /mybucket
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/mybucket".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);

    // 2. PutObject PUT /mybucket/data/file1.csv
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/mybucket/data/file1.csv".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: b"col1,col2\nval1,val2\n".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    assert!(res.headers.contains_key("etag"));

    // 3. ListObjectsV2 GET /mybucket?prefix=data/
    let mut query = HashMap::new();
    query.insert("prefix".into(), "data/".into());
    let req = HttpRequest {
        method: "GET".into(),
        path: "/mybucket".into(),
        raw_query: "prefix=data/".into(),
        query,
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    let xml = String::from_utf8(res.body).unwrap();
    assert!(xml.contains("<ListBucketResult"));
    assert!(xml.contains("<Key>data/file1.csv</Key>"));

    // 4. GetObject GET /mybucket/data/file1.csv
    let req = HttpRequest {
        method: "GET".into(),
        path: "/mybucket/data/file1.csv".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.body, b"col1,col2\nval1,val2\n");

    // 5. Multipart Upload Flow (§22.3)
    // 5a. InitiateMultipartUpload: POST /mybucket/large.bin?uploads
    let mut query = HashMap::new();
    query.insert("uploads".into(), "".into());
    let req = HttpRequest {
        method: "POST".into(),
        path: "/mybucket/large.bin".into(),
        raw_query: "uploads".into(),
        query,
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    let xml = String::from_utf8(res.body).unwrap();
    let upload_id = xml
        .split("<UploadId>")
        .nth(1)
        .unwrap()
        .split("</UploadId>")
        .next()
        .unwrap();

    // 5b. UploadPart 1: PUT /mybucket/large.bin?uploadId=...&partNumber=1
    let mut query = HashMap::new();
    query.insert("uploadId".into(), upload_id.to_string());
    query.insert("partNumber".into(), "1".into());
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/mybucket/large.bin".into(),
        raw_query: format!("uploadId={}&partNumber=1", upload_id),
        query,
        headers: HashMap::new(),
        body: b"Part 1 Data; ".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);

    // 5c. UploadPart 2: PUT /mybucket/large.bin?uploadId=...&partNumber=2
    let mut query = HashMap::new();
    query.insert("uploadId".into(), upload_id.to_string());
    query.insert("partNumber".into(), "2".into());
    let req = HttpRequest {
        method: "PUT".into(),
        path: "/mybucket/large.bin".into(),
        raw_query: format!("uploadId={}&partNumber=2", upload_id),
        query,
        headers: HashMap::new(),
        body: b"Part 2 Data.".to_vec(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);

    // 5d. CompleteMultipartUpload: POST /mybucket/large.bin?uploadId=...
    let mut query = HashMap::new();
    query.insert("uploadId".into(), upload_id.to_string());
    let req = HttpRequest {
        method: "POST".into(),
        path: "/mybucket/large.bin".into(),
        raw_query: format!("uploadId={}", upload_id),
        query,
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);

    // Verify assembled content
    let final_bytes = fs.read_bytes("/mybucket/large.bin").await.unwrap();
    assert_eq!(final_bytes, b"Part 1 Data; Part 2 Data.");
}

#[tokio::test]
async fn test_nfs_stateless_handles_and_estale() {
    let fs = setup_test_fs().await;
    let nfs = NfsServer::new(fs.clone());

    // 1. File handle wire serialization
    let handle = NfsFileHandle::new("/docs/report.pdf", 42);
    let bytes = handle.to_bytes();
    let decoded = NfsFileHandle::from_bytes(&bytes).expect("Failed to parse handle");
    assert_eq!(handle, decoded);

    // 2. Create file and get handle
    let root_handle = NfsFileHandle::new("/", 0);
    let (file_handle, fattr) = nfs
        .create(&root_handle, "test_file.txt")
        .await
        .expect("create failed");
    assert_eq!(file_handle.path, "/test_file.txt");
    assert_eq!(fattr.size, 0);

    // 3. Write data
    let (written, fattr_after) = nfs
        .write(&file_handle, 0, b"Hello NFSv3")
        .await
        .expect("write failed");
    assert_eq!(written, 11);
    assert_eq!(fattr_after.size, 11);

    // 4. Stale handle verification (§22.2)
    // Modify file externally to bump generation
    fs.write_text("/test_file.txt", "New External Generation", None)
        .await
        .unwrap();

    // Old handle should now return ESTALE
    let stale_res = nfs.getattr(&file_handle).await;
    assert_eq!(stale_res.err(), Some(NFS3ERR_STALE));

    // 5. Mount command generation
    let mount_cmd = nfs.mount_command("/mnt/surrealfs", 2049);
    assert!(mount_cmd.contains("mount -t nfs"));
    assert!(mount_cmd.contains("/mnt/surrealfs"));
}

#[tokio::test]
async fn test_sftp_packet_subsystem() {
    let fs = setup_test_fs().await;
    let sftp = SftpSession::new(fs.clone());

    // 1. SSH_FXP_INIT
    let init_pkt = vec![SSH_FXP_INIT, 0, 0, 0, 3];
    let resp = sftp.process_packet(&init_pkt).await;
    assert_eq!(resp[4], SSH_FXP_VERSION);

    // 2. SSH_FXP_REALPATH "/"
    let mut realpath_pkt = vec![SSH_FXP_REALPATH];
    realpath_pkt.extend_from_slice(&1u32.to_be_bytes()); // id = 1
    let path = "/".as_bytes();
    realpath_pkt.extend_from_slice(&(path.len() as u32).to_be_bytes());
    realpath_pkt.extend_from_slice(path);
    let resp = sftp.process_packet(&realpath_pkt).await;
    assert_eq!(resp[4], SSH_FXP_NAME);

    // 3. SSH_FXP_MKDIR "/incoming"
    let mut mkdir_pkt = vec![SSH_FXP_MKDIR];
    mkdir_pkt.extend_from_slice(&2u32.to_be_bytes()); // id = 2
    let path = "/incoming".as_bytes();
    mkdir_pkt.extend_from_slice(&(path.len() as u32).to_be_bytes());
    mkdir_pkt.extend_from_slice(path);
    let resp = sftp.process_packet(&mkdir_pkt).await;
    assert_eq!(resp[4], SSH_FXP_STATUS);

    // 4. SSH_FXP_OPEN "/incoming/drop.txt"
    let mut open_pkt = vec![SSH_FXP_OPEN];
    open_pkt.extend_from_slice(&3u32.to_be_bytes()); // id = 3
    let path = "/incoming/drop.txt".as_bytes();
    open_pkt.extend_from_slice(&(path.len() as u32).to_be_bytes());
    open_pkt.extend_from_slice(path);
    let resp = sftp.process_packet(&open_pkt).await;
    assert_eq!(resp[4], SSH_FXP_HANDLE);
    let handle_len = u32::from_be_bytes(resp[9..13].try_into().unwrap()) as usize;
    let handle_str = String::from_utf8(resp[13..13 + handle_len].to_vec()).unwrap();

    // 5. SSH_FXP_WRITE data
    let mut write_pkt = vec![SSH_FXP_WRITE];
    write_pkt.extend_from_slice(&4u32.to_be_bytes()); // id = 4
    write_pkt.extend_from_slice(&(handle_str.len() as u32).to_be_bytes());
    write_pkt.extend_from_slice(handle_str.as_bytes());
    write_pkt.extend_from_slice(&0u64.to_be_bytes()); // offset = 0
    let data = b"SFTP Dropped Payload";
    write_pkt.extend_from_slice(&(data.len() as u32).to_be_bytes());
    write_pkt.extend_from_slice(data);
    let resp = sftp.process_packet(&write_pkt).await;
    assert_eq!(resp[4], SSH_FXP_STATUS);

    // 6. SSH_FXP_CLOSE
    let mut close_pkt = vec![SSH_FXP_CLOSE];
    close_pkt.extend_from_slice(&5u32.to_be_bytes());
    close_pkt.extend_from_slice(&(handle_str.len() as u32).to_be_bytes());
    close_pkt.extend_from_slice(handle_str.as_bytes());
    let resp = sftp.process_packet(&close_pkt).await;
    assert_eq!(resp[4], SSH_FXP_STATUS);

    // Verify file content in SurrealFS
    let content = fs.read_text("/incoming/drop.txt").await.unwrap();
    assert_eq!(content, "SFTP Dropped Payload");
}

#[tokio::test]
async fn test_credentials_and_authentication() {
    let fs = setup_test_fs().await;
    let auth = Authenticator::new(fs.clone());

    // 1. Create external credentials (§22)
    fs.create_credential(
        "webdav",
        "alice",
        Some("secret123"),
        "user:alice",
        None,
    )
    .await
    .expect("Failed to create credential");

    // 2. Validate Basic Auth
    // Header format: Basic base64(alice:secret123)
    let basic_token = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"alice:secret123",
    );
    let header_val = format!("Basic {}", basic_token);
    let user = auth
        .authenticate_basic(&header_val)
        .await
        .expect("Authentication should succeed");
    assert_eq!(user.user_id, "user:alice");
    assert_eq!(user.identifier, "alice");

    // Invalid password
    let bad_token = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"alice:wrongpass",
    );
    let bad_header = format!("Basic {}", bad_token);
    assert!(auth.authenticate_basic(&bad_header).await.is_err());

    // 3. Revoke credential
    fs.revoke_credential("webdav", "alice").await.unwrap();
    assert!(auth.authenticate_basic(&header_val).await.is_err());
}
