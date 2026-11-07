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

static SERVER_URL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
static SERVER_PROC: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

async fn server_url() -> String {
    SERVER_URL
        .get_or_init(|| async {
            if let Ok(u) = std::env::var("SURREALFS_TEST_URL") {
                return u;
            }
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .expect("bind a free port")
                .local_addr()
                .unwrap()
                .port();
            let proc = std::process::Command::new("surreal")
                .args([
                    "start",
                    "--allow-all",
                    "-u",
                    "root",
                    "-p",
                    "root",
                    "--bind",
                    &format!("127.0.0.1:{port}"),
                    "memory",
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("Failed to start surreal process. Ensure `surreal` is on PATH.");
            *SERVER_PROC.lock().unwrap() = Some(proc);
            for _ in 0..100 {
                if tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .is_ok()
                {
                    return format!("127.0.0.1:{port}/rpc");
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            panic!("SurrealDB server failed to become ready on port {port}");
        })
        .await
        .clone()
}

async fn setup_test_fs() -> SurrealFs {
    let url = server_url().await;
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
        method: "GET".into(),
        path: "/docs/hello.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers,
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 206);
    assert_eq!(res.into_bytes().await, b"Hello");

    // 6. Concurrency control: PUT with If-Match
    let mut headers = HashMap::new();
    headers.insert("if-match".into(), "\"gen-9999\"".into());
    let req = HttpRequest {
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
        method: "GET".into(),
        path: "/mybucket/data/file1.csv".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = server.handle(req).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.into_bytes().await, b"col1,col2\nval1,val2\n");

    // 5. Multipart Upload Flow (§22.3)
    // 5a. InitiateMultipartUpload: POST /mybucket/large.bin?uploads
    let mut query = HashMap::new();
    query.insert("uploads".into(), "".into());
    let req = HttpRequest {
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
        body_stream: None,
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
    fs.create_credential("webdav", "alice", Some("secret123"), "user:alice", None)
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

#[tokio::test]
async fn test_unified_gateway_api_mcp_and_rpc() {
    let fs = setup_test_fs().await;
    let gateway = surrealfs_server::SurrealFsGateway::new(fs.clone());

    // 1. Write file via REST API (/api/file)
    let req = HttpRequest {
        body_stream: None,
        method: "POST".into(),
        path: "/api/file".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::to_vec(&serde_json::json!({
            "path": "/notes/hello.md",
            "content": "# Hello Gateway\n\nUnified Layer 2 test.",
            "mkdir_parents": true
        }))
        .unwrap(),
    };
    let res = gateway.handle(req).await;
    assert_eq!(res.status, 200);

    // 2. Read file via REST API (/api/file?path=/notes/hello.md)
    let mut query = HashMap::new();
    query.insert("path".into(), "/notes/hello.md".into());
    let req = HttpRequest {
        body_stream: None,
        method: "GET".into(),
        path: "/api/file".into(),
        raw_query: "path=/notes/hello.md".into(),
        query,
        headers: HashMap::new(),
        body: Vec::new(),
    };
    let res = gateway.handle(req).await;
    assert_eq!(res.status, 200);
    let body_json: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
    assert_eq!(
        body_json.get("content").and_then(|v| v.as_str()),
        Some("# Hello Gateway\n\nUnified Layer 2 test.")
    );

    // 3. Streamable HTTP MCP (/mcp): tools/list + tools/call (cat)
    let mcp_list_req = HttpRequest {
        body_stream: None,
        method: "POST".into(),
        path: "/mcp".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .unwrap(),
    };
    let res = gateway.handle(mcp_list_req).await;
    assert_eq!(res.status, 200);
    let list_json: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
    let tools = list_json["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 15);

    let mcp_call_req = HttpRequest {
        body_stream: None,
        method: "POST".into(),
        path: "/mcp".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "cat",
                "arguments": { "path": "/notes/hello.md" }
            }
        }))
        .unwrap(),
    };
    let res = gateway.handle(mcp_call_req).await;
    assert_eq!(res.status, 200);
    let call_json: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
    let text = call_json["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Hello Gateway"));

    // 4. Client RPC proxy (/api/rpc)
    let rpc_req = HttpRequest {
        body_stream: None,
        method: "POST".into(),
        path: "/api/rpc".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::to_vec(&serde_json::json!({
            "sql": "RETURN fn::sfs_read($path, $caller);",
            "vars": { "path": "/notes/hello.md" }
        }))
        .unwrap(),
    };
    let res = gateway.handle(rpc_req).await;
    assert_eq!(res.status, 200);
    let rpc_json: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
    assert_eq!(
        rpc_json["result"][0]["result"].as_str(),
        Some("# Hello Gateway\n\nUnified Layer 2 test.")
    );
}

#[tokio::test]
async fn test_s3_sigv4_hmac_and_webdav_lock_isolation() {
    let fs = setup_test_fs().await;
    let auth = Authenticator::new(fs.clone());

    // 1. Secretless credential must NOT pass Basic Auth even with empty password
    fs.create_credential("ssh", "pubkey-only", None, "user:ssh_only", None)
        .await
        .unwrap();
    let empty_pass_token =
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"pubkey-only:");
    assert!(auth
        .authenticate_basic(&format!("Basic {}", empty_pass_token))
        .await
        .is_err());

    // 2. S3 SigV4 HMAC verification
    fs.create_credential(
        "s3",
        "AKIAIOSFODNN7EXAMPLE",
        Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        "user:s3_alice",
        None,
    )
    .await
    .unwrap();

    let sig = Authenticator::compute_s3_signature(
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "20261015",
        "us-east-1",
        "s3",
    );
    let valid_s3_hdr = format!(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20261015/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-date, Signature={}",
        sig
    );
    let s3_user = auth
        .authenticate_s3(Some(&valid_s3_hdr))
        .await
        .expect("Valid SigV4 signature should succeed");
    assert_eq!(s3_user.user_id, "user:s3_alice");

    let forged_s3_hdr = "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20261015/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-date, Signature=0000000000000000000000000000000000000000000000000000000000000000";
    assert!(auth.authenticate_s3(Some(forged_s3_hdr)).await.is_err());

    // 3. WebDAV lock holder isolation & 0-byte Range guard
    let webdav = WebDavServer::new(fs.clone(), "/").with_require_auth(true);

    // Unauthenticated PUT rejected when require_auth = true
    let unauth_put = HttpRequest {
        body_stream: None,
        method: "PUT".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: HashMap::new(),
        body: Vec::new(),
    };
    assert_eq!(webdav.handle(unauth_put).await.status, 401);

    // Provision alice and bob WebDAV credentials
    fs.create_credential("webdav", "alice", Some("passA"), "user:alice", None)
        .await
        .unwrap();
    fs.create_credential("webdav", "bob", Some("passB"), "user:bob", None)
        .await
        .unwrap();
    let alice_auth = format!(
        "Basic {}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"alice:passA")
    );
    let bob_auth = format!(
        "Basic {}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"bob:passB")
    );

    let mut headers = HashMap::new();
    headers.insert("authorization".into(), alice_auth.clone());
    let put_empty = HttpRequest {
        body_stream: None,
        method: "PUT".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: headers.clone(),
        body: Vec::new(),
    };
    assert_eq!(webdav.handle(put_empty).await.status, 201);

    // Range request on 0-byte file does not underflow or return invalid 206
    let mut range_headers = headers.clone();
    range_headers.insert("range".into(), "bytes=0-10".into());
    let get_empty = HttpRequest {
        body_stream: None,
        method: "GET".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: range_headers,
        body: Vec::new(),
    };
    let get_res = webdav.handle(get_empty).await;
    assert_eq!(get_res.status, 200);
    assert!(get_res.body.is_empty());

    // Make /empty.txt writable so Bob tests lock contention (423) rather than file mode (403)
    fs.with_caller("user:alice")
        .chmod("/empty.txt", 0o666)
        .await
        .unwrap();

    // Alice locks /empty.txt; Bob cannot acquire or release lock while Alice holds it
    let lock_alice = HttpRequest {
        body_stream: None,
        method: "LOCK".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: headers.clone(),
        body: Vec::new(),
    };
    assert_eq!(webdav.handle(lock_alice).await.status, 200);

    let mut bob_headers = HashMap::new();
    bob_headers.insert("authorization".into(), bob_auth);
    let lock_bob = HttpRequest {
        body_stream: None,
        method: "LOCK".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: bob_headers.clone(),
        body: Vec::new(),
    };
    assert_eq!(webdav.handle(lock_bob).await.status, 423);

    let unlock_bob = HttpRequest {
        body_stream: None,
        method: "UNLOCK".into(),
        path: "/empty.txt".into(),
        raw_query: "".into(),
        query: HashMap::new(),
        headers: bob_headers,
        body: Vec::new(),
    };
    assert_eq!(webdav.handle(unlock_bob).await.status, 423);
}

#[tokio::test]
async fn test_nfs_rpc_call_and_sftp_pflags() {
    let fs = setup_test_fs().await;
    fs.write_text("/hello.txt", "Hello World", None)
        .await
        .unwrap();

    // 1. NFS ONC RPC call framing (MOUNTPROC3_MNT + NFSPROC3_LOOKUP)
    let nfs = NfsServer::new(fs.clone());
    let mut mnt_call = Vec::new();
    mnt_call.extend_from_slice(&100u32.to_be_bytes()); // xid
    mnt_call.extend_from_slice(&0u32.to_be_bytes()); // CALL
    mnt_call.extend_from_slice(&2u32.to_be_bytes()); // rpcvers = 2
    mnt_call.extend_from_slice(&100005u32.to_be_bytes()); // MOUNT prog
    mnt_call.extend_from_slice(&3u32.to_be_bytes()); // vers = 3
    mnt_call.extend_from_slice(&1u32.to_be_bytes()); // proc = MNT (1)
    mnt_call.extend_from_slice(&[0u8; 16]); // AUTH_NULL cred + verf
    let mnt_reply = nfs.handle_rpc_call(&mnt_call).await.unwrap();
    assert_eq!(u32::from_be_bytes(mnt_reply[0..4].try_into().unwrap()), 100);
    assert_eq!(u32::from_be_bytes(mnt_reply[20..24].try_into().unwrap()), 0); // MNT3_OK

    // 2. SFTP partial write without SSH_FXF_TRUNC preserves existing prefix
    let sftp = SftpSession::new(fs.clone());
    let mut open_pkt = vec![SSH_FXP_OPEN];
    open_pkt.extend_from_slice(&10u32.to_be_bytes());
    let p = b"/hello.txt";
    open_pkt.extend_from_slice(&(p.len() as u32).to_be_bytes());
    open_pkt.extend_from_slice(p);
    open_pkt.extend_from_slice(&SSH_FXF_WRITE.to_be_bytes()); // write without trunc
    let resp = sftp.process_packet(&open_pkt).await;
    assert_eq!(resp[4], SSH_FXP_HANDLE);
    let h_len = u32::from_be_bytes(resp[9..13].try_into().unwrap()) as usize;
    let handle_str = String::from_utf8(resp[13..13 + h_len].to_vec()).unwrap();

    // Overwrite at offset 6 ("World" -> "SFTP!")
    let mut write_pkt = vec![SSH_FXP_WRITE];
    write_pkt.extend_from_slice(&11u32.to_be_bytes());
    write_pkt.extend_from_slice(&(handle_str.len() as u32).to_be_bytes());
    write_pkt.extend_from_slice(handle_str.as_bytes());
    write_pkt.extend_from_slice(&6u64.to_be_bytes());
    let chunk = b"SFTP!";
    write_pkt.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
    write_pkt.extend_from_slice(chunk);
    sftp.process_packet(&write_pkt).await;

    let mut close_pkt = vec![SSH_FXP_CLOSE];
    close_pkt.extend_from_slice(&12u32.to_be_bytes());
    close_pkt.extend_from_slice(&(handle_str.len() as u32).to_be_bytes());
    close_pkt.extend_from_slice(handle_str.as_bytes());
    sftp.process_packet(&close_pkt).await;

    let updated = fs.read_text("/hello.txt").await.unwrap();
    assert_eq!(updated, "Hello SFTP!");
}

#[tokio::test]
async fn test_gateway_token_gates_requests_and_ignores_bad_tokens() {
    let fs = setup_test_fs().await;
    let gateway = surrealfs_server::SurrealFsGateway::new(fs)
        .with_server_token(Some("s3cret-token".to_string()));
    let req = |auth: Option<&str>, user: Option<&str>| {
        let mut headers = HashMap::new();
        if let Some(a) = auth {
            headers.insert("authorization".to_string(), a.to_string());
        }
        if let Some(u) = user {
            headers.insert("x-surrealfs-user".to_string(), u.to_string());
        }
        HttpRequest {
            body_stream: None,
            method: "GET".into(),
            path: "/api/ls".into(),
            raw_query: "".into(),
            query: HashMap::new(),
            headers,
            body: Vec::new(),
        }
    };
    assert_eq!(gateway.handle(req(None, Some("root"))).await.status, 401);
    assert_eq!(
        gateway
            .handle(req(Some("Bearer nope"), Some("root")))
            .await
            .status,
        401
    );
    assert_ne!(
        gateway
            .handle(req(Some("Bearer s3cret-token"), Some("alice")))
            .await
            .status,
        401
    );
}

async fn http_exchange(addr: &str, head: &str, body: &[u8]) -> (String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    (
        String::from_utf8_lossy(&raw[..split]).to_string(),
        raw[split + 4..].to_vec(),
    )
}

#[tokio::test]
async fn test_large_bodies_stream_in_and_out_over_tcp() {
    use surrealfs_core::MemoryBlobStore;
    let store = std::sync::Arc::new(MemoryBlobStore::new());
    let fs = setup_test_fs().await.with_blob_store(store.clone());
    let webdav = WebDavServer::new(fs.clone(), "/");

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr = format!("127.0.0.1:{port}");
    let serve_addr = addr.clone();
    tokio::spawn(async move {
        let _ = surrealfs_server::serve_http(&serve_addr, webdav).await;
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(&addr).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let mut data = Vec::new();
    for i in 0..400_000u32 {
        data.extend_from_slice(format!("{i} {}\n", i.wrapping_mul(2654435761)).as_bytes());
    }
    assert!(data.len() > 4 * 1024 * 1024);

    let (head, _) = http_exchange(
        &addr,
        &format!(
            "PUT /big/data.bin HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            data.len()
        ),
        &data,
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 201"), "{head}");
    assert!(
        !store.is_empty(),
        "the body was chunked into the blob store"
    );
    assert_eq!(
        fs.stat("/big/data.bin").await.unwrap().unwrap().size as usize,
        data.len()
    );

    let (head, body) =
        http_exchange(&addr, "GET /big/data.bin HTTP/1.1\r\nHost: x\r\n\r\n", b"").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head
        .to_lowercase()
        .contains(&format!("content-length: {}", data.len())));
    assert_eq!(body, data);

    let n = data.len();
    let (head, body) = http_exchange(
        &addr,
        "GET /big/data.bin HTTP/1.1\r\nHost: x\r\nRange: bytes=1000000-1999999\r\n\r\n",
        b"",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 206"), "{head}");
    assert!(head
        .to_lowercase()
        .contains(&format!("content-range: bytes 1000000-1999999/{n}")));
    assert_eq!(body, &data[1_000_000..2_000_000]);

    let (head, body) = http_exchange(
        &addr,
        "GET /big/data.bin HTTP/1.1\r\nHost: x\r\nRange: bytes=-100\r\n\r\n",
        b"",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 206"), "{head}");
    assert_eq!(body, &data[n - 100..]);

    let (head, _) = http_exchange(
        &addr,
        &format!("GET /big/data.bin HTTP/1.1\r\nHost: x\r\nRange: bytes={n}-\r\n\r\n"),
        b"",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 416"), "{head}");
    assert!(head
        .to_lowercase()
        .contains(&format!("content-range: bytes */{n}")));

    let (head, body) =
        http_exchange(&addr, "HEAD /big/data.bin HTTP/1.1\r\nHost: x\r\n\r\n", b"").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head
        .to_lowercase()
        .contains(&format!("content-length: {n}")));
    assert!(body.is_empty());

    // A body that stops short is not committed.
    let (head, _) = http_exchange(
        &addr,
        &format!(
            "PUT /big/short.bin HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
            data.len()
        ),
        &data[..2_000_000],
    )
    .await;
    assert!(!head.starts_with("HTTP/1.1 201"), "{head}");
    assert!(fs.stat("/big/short.bin").await.unwrap().is_none());
}
