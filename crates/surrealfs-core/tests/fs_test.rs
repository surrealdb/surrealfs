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
