mod common;

use common::create_test_fs;
use serde_json::json;

#[tokio::test]
async fn test_read_and_write_roundtrip() {
    let fs = create_test_fs().await;

    let res = fs
        .write_text("/notes/hello.txt", "Hello SurrealFS from Rust!", None)
        .await
        .expect("write_text failed");
    assert_eq!(res.path, "/notes/hello.txt");
    assert_eq!(res.filename, "hello.txt");
    assert_eq!(res.size, 26);

    let content = fs
        .read_text("/notes/hello.txt")
        .await
        .expect("read_text failed");
    assert_eq!(content, "Hello SurrealFS from Rust!");
}

#[tokio::test]
async fn test_stat_and_exists() {
    let fs = create_test_fs().await;

    assert!(!fs.exists("/test.txt").await.unwrap());
    assert!(fs.stat("/test.txt").await.unwrap().is_none());

    fs.write_text("/test.txt", "Rust core testing", None)
        .await
        .unwrap();

    assert!(fs.exists("/test.txt").await.unwrap());
    let entry = fs.stat("/test.txt").await.unwrap().expect("stat was none");
    assert_eq!(entry.path, "/test.txt");
    assert_eq!(entry.size, 17);
    assert!(!entry.is_folder);
}

#[tokio::test]
async fn test_append_and_edit() {
    let fs = create_test_fs().await;

    fs.write_text("/doc.md", "Line 1\n", None).await.unwrap();

    fs.append_text("/doc.md", "Line 2\n").await.unwrap();
    let text = fs.read_text("/doc.md").await.unwrap();
    assert_eq!(text, "Line 1\nLine 2\n");

    fs.edit_text("/doc.md", "Line 1", "First Line", None)
        .await
        .unwrap();
    let text2 = fs.read_text("/doc.md").await.unwrap();
    assert_eq!(text2, "First Line\nLine 2\n");
}

#[tokio::test]
async fn test_mkdir_and_ls() {
    let fs = create_test_fs().await;

    fs.mkdir("/deep/nested/dir", true).await.unwrap();
    fs.write_text("/deep/nested/dir/a.txt", "a", None)
        .await
        .unwrap();
    fs.write_text("/deep/nested/dir/b.txt", "b", None)
        .await
        .unwrap();

    let list = fs.ls("/deep/nested/dir").await.unwrap();
    assert_eq!(list.len(), 2);
    let names: Vec<String> = list.into_iter().map(|e| e.filename).collect();
    assert!(names.contains(&"a.txt".to_string()));
    assert!(names.contains(&"b.txt".to_string()));
}

#[tokio::test]
async fn test_mv_and_cp() {
    let fs = create_test_fs().await;

    fs.write_text("/original.txt", "payload", None)
        .await
        .unwrap();

    fs.cp("/original.txt", "/copied.txt", false).await.unwrap();
    assert_eq!(fs.read_text("/copied.txt").await.unwrap(), "payload");
    assert!(fs.exists("/original.txt").await.unwrap());

    fs.mv("/copied.txt", "/moved.txt").await.unwrap();
    assert!(!fs.exists("/copied.txt").await.unwrap());
    assert_eq!(fs.read_text("/moved.txt").await.unwrap(), "payload");
}

#[tokio::test]
async fn test_rm_file_and_dir() {
    let fs = create_test_fs().await;

    fs.write_text("/temp/f.txt", "remove me", None)
        .await
        .unwrap();
    assert!(fs.exists("/temp/f.txt").await.unwrap());

    fs.rm("/temp/f.txt", false).await.unwrap();
    assert!(!fs.exists("/temp/f.txt").await.unwrap());

    fs.mkdir("/remove_dir", false).await.unwrap();
    fs.rm("/remove_dir", false).await.unwrap();
    assert!(!fs.exists("/remove_dir").await.unwrap());
}

#[tokio::test]
async fn test_history_and_restore() {
    let fs = create_test_fs().await;

    fs.write_text("/versioned.txt", "Gen 1", None)
        .await
        .unwrap();
    fs.write_text("/versioned.txt", "Gen 2", None)
        .await
        .unwrap();
    fs.write_text("/versioned.txt", "Gen 3", None)
        .await
        .unwrap();

    let hist = fs.history("/versioned.txt", Some(10)).await.unwrap();
    assert!(hist.len() >= 2);

    fs.restore("/versioned.txt", 1).await.unwrap();
    let restored = fs.read_text("/versioned.txt").await.unwrap();
    assert_eq!(restored, "Gen 1");
}

#[tokio::test]
async fn test_advisory_locks() {
    let fs = create_test_fs().await;

    fs.write_text("/locked.md", "critical doc", None)
        .await
        .unwrap();
    fs.chmod("/locked.md", 0o666).await.unwrap();

    let lock = fs
        .acquire_lock("/locked.md", 60, "editing", "agent-alpha")
        .await
        .unwrap();
    assert_eq!(lock.holder, "agent-alpha");

    let locks = fs.list_locks().await.unwrap();
    assert!(locks.iter().any(|l| l.holder == "agent-alpha"));

    fs.release_lock("/locked.md", "agent-alpha").await.unwrap();
    let locks_after = fs.list_locks().await.unwrap();
    assert!(!locks_after.iter().any(|l| l.holder == "agent-alpha"));
}

#[tokio::test]
async fn test_zero_copy_workspace() {
    let fs = create_test_fs().await;

    fs.write_text("/shared/task.md", "Base instructions", None)
        .await
        .unwrap();

    // Fork
    fs.fork_workspace("main", "feature-x", "agent-1")
        .await
        .unwrap();

    // Diff
    let diff = fs.diff_workspace("feature-x", "agent-1").await.unwrap();
    assert_eq!(diff.len(), 1);

    // Merge
    let merge_res = fs
        .merge_workspace("feature-x", "main", "agent-1")
        .await
        .unwrap();
    assert_eq!(merge_res.get("status").and_then(|v| v.as_str()), Some("ok"));
}

#[tokio::test]
async fn test_agent_mailbox() {
    let fs = create_test_fs().await;

    let msg_id = fs
        .send_mailbox(
            "worker-agent",
            "summarize",
            json!({"path": "/notes/meeting.md"}),
            10,
        )
        .await
        .unwrap();
    assert!(!msg_id.is_empty());

    let claimed = fs
        .claim_mailbox("worker-agent", "worker-node-1", 30)
        .await
        .unwrap()
        .expect("should claim pending message");
    assert_eq!(claimed.agent_id, "worker-agent");
    assert_eq!(claimed.op, "summarize");

    fs.complete_mailbox(&claimed.id, json!({"summary": "Approved"}))
        .await
        .unwrap();
}

#[tokio::test]
async fn test_grep() {
    let fs = create_test_fs().await;

    fs.write_text(
        "/src/app.py",
        "def login():\n    # TODO: fix auth\n    pass\n",
        None,
    )
    .await
    .unwrap();
    fs.write_text("/src/utils.py", "def helper():\n    return 42\n", None)
        .await
        .unwrap();

    let matches = fs.grep("TODO", None, false).await.unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, "/src/app.py");
    assert_eq!(matches[0].line_number, 2);
    assert!(matches[0].line_text.contains("TODO: fix auth"));
}

#[tokio::test]
async fn test_write_and_read_bytes_roundtrip() {
    let fs = create_test_fs().await;

    let raw: Vec<u8> = vec![0x00, 0xFF, 0x10, 0x7F, 0x80, 0xDE, 0xAD, 0xBE, 0xEF];
    let entry = fs
        .write_bytes("/bin/payload.bin", &raw, None)
        .await
        .expect("write_bytes failed");
    assert_eq!(entry.path, "/bin/payload.bin");
    assert_eq!(entry.size, raw.len() as u64);
    assert_eq!(entry.generation, 1);

    let read_back = fs
        .read_bytes("/bin/payload.bin")
        .await
        .expect("read_bytes failed");
    assert_eq!(read_back, raw);
}

#[tokio::test]
async fn test_optimistic_concurrency_if_generation() {
    use surrealfs_core::SurrealFsError;

    let fs = create_test_fs().await;

    // Creating a new file with Some(1) must conflict because the file doesn't exist yet
    let err = fs
        .write_text("/concurrency/doc.txt", "v1", Some(1))
        .await
        .unwrap_err();
    assert!(
        matches!(err, SurrealFsError::Conflict(_)),
        "expected Conflict on non-existent file with if_generation=Some(1), got {:?}",
        err
    );

    let err_bytes = fs
        .write_bytes("/concurrency/bin.dat", b"abc", Some(1))
        .await
        .unwrap_err();
    assert!(
        matches!(err_bytes, SurrealFsError::Conflict(_)),
        "expected Conflict on non-existent binary file with if_generation=Some(1), got {:?}",
        err_bytes
    );

    // Create v1
    let e1 = fs
        .write_text("/concurrency/doc.txt", "line1\n", None)
        .await
        .unwrap();
    assert_eq!(e1.generation, 1);

    // Updating with matching generation 1 succeeds -> generation 2
    let e2 = fs
        .write_text("/concurrency/doc.txt", "line1\nline2\n", Some(1))
        .await
        .unwrap();
    assert_eq!(e2.generation, 2);

    // Updating with stale generation 1 fails with Conflict
    let stale_write = fs
        .write_text("/concurrency/doc.txt", "overwrite", Some(1))
        .await
        .unwrap_err();
    assert!(matches!(stale_write, SurrealFsError::Conflict(_)));

    // append_text_if_generation with stale generation 1 fails, with generation 2 succeeds -> generation 3
    let stale_append = fs
        .append_text_if_generation("/concurrency/doc.txt", "line3\n", Some(1))
        .await
        .unwrap_err();
    assert!(matches!(stale_append, SurrealFsError::Conflict(_)));

    let e3 = fs
        .append_text_if_generation("/concurrency/doc.txt", "line3\n", Some(2))
        .await
        .unwrap();
    assert_eq!(e3.generation, 3);

    // edit_text with stale generation 2 fails, with generation 3 succeeds -> generation 4
    let stale_edit = fs
        .edit_text("/concurrency/doc.txt", "line1", "LINE_ONE", Some(2))
        .await
        .unwrap_err();
    assert!(matches!(stale_edit, SurrealFsError::Conflict(_)));

    let e4 = fs
        .edit_text("/concurrency/doc.txt", "line1", "LINE_ONE", Some(3))
        .await
        .unwrap();
    assert_eq!(e4.generation, 4);

    // write_bytes with matching generation vs stale generation
    let b1 = fs
        .write_bytes("/concurrency/bin.dat", b"v1", None)
        .await
        .unwrap();
    assert_eq!(b1.generation, 1);

    let stale_b = fs
        .write_bytes("/concurrency/bin.dat", b"v2", Some(99))
        .await
        .unwrap_err();
    assert!(matches!(stale_b, SurrealFsError::Conflict(_)));

    let b2 = fs
        .write_bytes("/concurrency/bin.dat", b"v2", Some(1))
        .await
        .unwrap();
    assert_eq!(b2.generation, 2);
}

#[tokio::test]
async fn test_posix_negative_paths() {
    use surrealfs_core::SurrealFsError;

    let fs = create_test_fs().await;

    // 1. NotFound on reading missing file
    let err = fs.read_text("/missing/nope.txt").await.unwrap_err();
    assert!(matches!(err, SurrealFsError::NotFound(_)), "got {:?}", err);

    // 2. mkdir without parents on missing intermediate directory -> NotFound
    let err = fs.mkdir("/no_such_parent/child", false).await.unwrap_err();
    assert!(matches!(err, SurrealFsError::NotFound(_)), "got {:?}", err);

    // 3. IsADirectory when reading/writing a folder as a file
    fs.mkdir("/dir_only", true).await.unwrap();
    let err = fs.read_text("/dir_only").await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::IsADirectory(_)),
        "expected IsADirectory on read_text(/dir_only), got {:?}",
        err
    );
    let err = fs.write_text("/dir_only", "text", None).await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::IsADirectory(_)),
        "expected IsADirectory on write_text(/dir_only), got {:?}",
        err
    );

    // 4. AlreadyExists when creating a folder with parents=false where one exists or over a file
    let err = fs.mkdir("/dir_only", false).await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::AlreadyExists(_)),
        "expected AlreadyExists on duplicate mkdir(/dir_only, false), got {:?}",
        err
    );

    // 5. DirectoryNotEmpty when removing non-empty folder with recursive=false
    fs.write_text("/dir_only/item.txt", "item", None)
        .await
        .unwrap();
    let err = fs.rm("/dir_only", false).await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::DirectoryNotEmpty(_)),
        "expected DirectoryNotEmpty on rm(/dir_only, false), got {:?}",
        err
    );

    // 6. InvalidPath when calling ls on a regular file
    let err = fs.ls("/dir_only/item.txt").await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::InvalidPath(_)),
        "expected InvalidPath on ls(/dir_only/item.txt), got {:?}",
        err
    );

    // 7. InvalidPath when moving a directory into its own subtree
    fs.mkdir("/dir_only/sub", true).await.unwrap();
    let err = fs.mv("/dir_only", "/dir_only/sub/nested").await.unwrap_err();
    assert!(
        matches!(err, SurrealFsError::InvalidPath(_)),
        "expected InvalidPath on moving directory into its own subtree, got {:?}",
        err
    );
}

#[tokio::test]
async fn test_multi_user_posix_permissions_and_group_acls() {
    use surrealfs_core::SurrealFsError;

    let fs = create_test_fs().await;
    let alice = fs.with_caller("alice");
    let bob = fs.with_caller("bob");
    let carol = fs.with_caller("carol");

    // Alice creates a private file and sets mode 0o600
    alice
        .write_text("/alice_secret.txt", "top secret", None)
        .await
        .unwrap();
    alice.chmod("/alice_secret.txt", 0o600).await.unwrap();

    // Alice can read & write; Bob gets PermissionDenied
    assert_eq!(
        alice.read_text("/alice_secret.txt").await.unwrap(),
        "top secret"
    );
    let bob_read = bob.read_text("/alice_secret.txt").await.unwrap_err();
    assert!(
        matches!(bob_read, SurrealFsError::PermissionDenied(_)),
        "expected PermissionDenied for bob read on 0o600, got {:?}",
        bob_read
    );
    let bob_write = bob
        .write_text("/alice_secret.txt", "hacked", None)
        .await
        .unwrap_err();
    assert!(
        matches!(bob_write, SurrealFsError::PermissionDenied(_)),
        "expected PermissionDenied for bob write on 0o600, got {:?}",
        bob_write
    );

    // Create group "eng" with member "bob" (not "carol")
    fs.create_group("eng", Some("Engineering")).await.unwrap();
    fs.add_group_member("eng", "bob").await.unwrap();

    // Assign group "eng" and mode 0o640 (owner rw, group r, other none)
    alice.chgrp("/alice_secret.txt", Some("eng")).await.unwrap();
    alice.chmod("/alice_secret.txt", 0o640).await.unwrap();

    // Bob (in group "eng") can now read, but still cannot write
    assert_eq!(
        bob.read_text("/alice_secret.txt").await.unwrap(),
        "top secret"
    );
    let bob_write2 = bob
        .write_text("/alice_secret.txt", "bob edit", None)
        .await
        .unwrap_err();
    assert!(matches!(bob_write2, SurrealFsError::PermissionDenied(_)));

    // Carol (not in "eng") cannot read
    let carol_read = carol.read_text("/alice_secret.txt").await.unwrap_err();
    assert!(matches!(carol_read, SurrealFsError::PermissionDenied(_)));

    // Upgrade mode to 0o660 (group rw): Bob can now write, Carol still denied
    alice.chmod("/alice_secret.txt", 0o660).await.unwrap();
    bob.write_text("/alice_secret.txt", "bob group write", None)
        .await
        .unwrap();
    assert_eq!(
        alice.read_text("/alice_secret.txt").await.unwrap(),
        "bob group write"
    );
    assert!(matches!(
        carol.read_text("/alice_secret.txt").await.unwrap_err(),
        SurrealFsError::PermissionDenied(_)
    ));

    // Mode 0o644: Carol (world/other) can read, but not write
    alice.chmod("/alice_secret.txt", 0o644).await.unwrap();
    assert_eq!(
        carol.read_text("/alice_secret.txt").await.unwrap(),
        "bob group write"
    );
    assert!(matches!(
        carol
            .write_text("/alice_secret.txt", "carol edit", None)
            .await
            .unwrap_err(),
        SurrealFsError::PermissionDenied(_)
    ));
}

#[tokio::test]
async fn test_ls_recursive_and_crdt_direct() {
    let fs = create_test_fs().await;

    fs.write_text("/tree/a/one.txt", "1", None).await.unwrap();
    fs.write_text("/tree/a/b/two.txt", "2", None).await.unwrap();
    fs.write_text("/tree/c/three.txt", "3", None).await.unwrap();

    let all = fs.ls_recursive("/tree").await.unwrap();
    let paths: Vec<String> = all.iter().map(|e| e.path.clone()).collect();
    assert!(paths.contains(&"/tree/a/one.txt".to_string()));
    assert!(paths.contains(&"/tree/a/b/two.txt".to_string()));
    assert!(paths.contains(&"/tree/c/three.txt".to_string()));

    // Direct CRDT lifecycle on a file
    fs.write_text("/collab.md", "Title\n", None).await.unwrap();
    fs.enable_crdt("/collab.md").await.unwrap();
    let enabled = fs.stat("/collab.md").await.unwrap().unwrap();
    assert!(enabled.crdt);

    fs.append_text("/collab.md", "Section A\n").await.unwrap();
    fs.edit_text("/collab.md", "Title", "# Main Title", None)
        .await
        .unwrap();
    let content = fs.read_text("/collab.md").await.unwrap();
    assert_eq!(content, "# Main Title\nSection A\n");

    fs.compact_crdt("/collab.md").await.unwrap();
    let compacted = fs.stat("/collab.md").await.unwrap().unwrap();
    assert!(compacted.crdt);
    assert_eq!(
        fs.read_text("/collab.md").await.unwrap(),
        "# Main Title\nSection A\n"
    );
}

#[tokio::test]
async fn test_full_text_semantic_hybrid_and_section_search() {
    use surrealfs_core::split_markdown_sections;

    let fs = create_test_fs().await;

    // Populate a corpus >= 10 documents so SurrealDB's unsmoothed BM25 IDF > 0
    for i in 0..8 {
        fs.write_text(
            &format!("/docs/filler_{}.md", i),
            &format!("Filler document number {} about routine gardening topics.", i),
            None,
        )
        .await
        .unwrap();
    }
    fs.write_text(
        "/docs/quantum.md",
        "# Quantum Computing\nSuperconducting qubits and error correction codes.\n",
        None,
    )
    .await
    .unwrap();
    fs.write_text(
        "/docs/networking.md",
        "# Network Protocols\nQUIC transport and zero-RTT connection establishment.\n",
        None,
    )
    .await
    .unwrap();

    // 1. Full-text search
    let fts_hits = fs
        .search("qubits", 5)
        .await
        .expect("FTS search failed");
    assert!(!fts_hits.is_empty());
    assert_eq!(fts_hits[0].entry.path, "/docs/quantum.md");
    assert!(fts_hits[0].snippet.to_lowercase().contains("qubits"));

    // 2. Vector embeddings (1536-dim as defined in file.surql)
    let mut vec_quantum = vec![0.0f32; 1536];
    vec_quantum[0] = 1.0;
    let mut vec_net = vec![0.0f32; 1536];
    vec_net[1] = 1.0;

    fs.set_embedding("/docs/quantum.md", &vec_quantum, "test-model-v1")
        .await
        .expect("set_embedding quantum failed");
    fs.set_embedding("/docs/networking.md", &vec_net, "test-model-v1")
        .await
        .expect("set_embedding networking failed");

    let sem_hits = fs
        .search_semantic(&vec_quantum, 5)
        .await
        .expect("search_semantic failed");
    assert!(!sem_hits.is_empty());
    assert_eq!(sem_hits[0].entry.path, "/docs/quantum.md");
    assert!(sem_hits[0].score > 0.9);

    // 3. Hybrid search (BM25 + HNSW RRF)
    let hyb_hits = fs
        .search_hybrid("qubits", Some(&vec_quantum), 5)
        .await
        .expect("search_hybrid failed");
    assert!(!hyb_hits.is_empty());
    assert_eq!(hyb_hits[0].entry.path, "/docs/quantum.md");

    // 4. Section indexing & search_sections
    let md_text = fs.read_text("/docs/quantum.md").await.unwrap();
    let parsed_sections = split_markdown_sections(&md_text);
    let sections_with_emb: Vec<_> = parsed_sections
        .into_iter()
        .map(|s| (s, Some(vec_quantum.clone())))
        .collect();
    let indexed_count = fs
        .index_file_sections("/docs/quantum.md", &sections_with_emb)
        .await
        .expect("index_file_sections failed");
    assert!(indexed_count >= 1);

    let sec_hits = fs
        .search_sections(&vec_quantum, 5)
        .await
        .expect("search_sections failed");
    assert!(!sec_hits.is_empty());
    assert_eq!(sec_hits[0].path, "/docs/quantum.md");
    assert!(sec_hits[0].content.contains("qubits"));
}

#[tokio::test]
async fn test_http_db_client_transport() {
    use surrealfs_core::{ConnectOptions, SurrealFs};

    let url = common::get_server_url().await;
    let ns = "test_http_transport".to_string();
    let db = "test".to_string();

    let ws_opts = ConnectOptions {
        url: url.to_string(),
        user: Some("root".to_string()),
        pass: Some("root".to_string()),
        ns: ns.clone(),
        db: db.clone(),
        caller: None,
    };

    let ws_fs = SurrealFs::connect(ws_opts.clone())
        .await
        .expect("SurrealFs::connect WS failed");
    ws_fs.apply_schema().await.expect("apply_schema failed");
    ws_fs
        .write_text("/from_ws.txt", "written over ws", None)
        .await
        .unwrap();

    // Connect to the exact same namespace/db over stateless HTTP RPC
    let http_fs = SurrealFs::connect_http(ws_opts).expect("SurrealFs::connect_http failed");

    // Verify reading existing file written via WS
    let text = http_fs.read_text("/from_ws.txt").await.unwrap();
    assert_eq!(text, "written over ws");

    // Verify writing and listing over HTTP
    http_fs
        .write_text("/from_http.txt", "written over http", None)
        .await
        .unwrap();
    let entries = http_fs.ls("/").await.unwrap();
    let names: Vec<String> = entries.into_iter().map(|e| e.filename).collect();
    assert!(names.contains(&"from_ws.txt".to_string()));
    assert!(names.contains(&"from_http.txt".to_string()));
}

