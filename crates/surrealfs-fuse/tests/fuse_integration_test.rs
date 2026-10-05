mod common;

use std::collections::HashMap;
use surrealfs_fuse::{
    SyntheticRouter, XattrEngine, SYNTHETIC_BRANCHES_DIR, SYNTHETIC_BRANCH_CURRENT,
    SYNTHETIC_LOCKS_DIR, SYNTHETIC_ROOT, SYNTHETIC_SEARCH_DIR, SYNTHETIC_STATS, SYNTHETIC_STATUS,
    SYNTHETIC_WHOAMI, XATTR_ACTION, XATTR_GENERATION, XATTR_LOCK, XATTR_LOCKED_BY, XATTR_OWNER,
};

#[tokio::test]
async fn test_synthetic_filesystem_operations() {
    let fs = common::create_test_fs().await;

    // Create some initial files
    fs.write_text(
        "/docs/system.md",
        "# System Architecture\nContains auth flow and database config.",
        None,
    )
    .await
    .unwrap();

    let router = SyntheticRouter::new("ws://127.0.0.1:8000".into(), "martin".into(), "main".into());

    // 1. Root lookup & readdir
    let synth_root = router.lookup(1, ".surrealfs").unwrap();
    assert_eq!(synth_root, SYNTHETIC_ROOT);

    let entries = router.readdir(SYNTHETIC_ROOT, &fs).await.unwrap();
    assert!(entries.iter().any(|(_, name, _)| name == "status"));
    assert!(entries.iter().any(|(_, name, _)| name == "whoami"));
    assert!(entries.iter().any(|(_, name, _)| name == "locks"));
    assert!(entries.iter().any(|(_, name, _)| name == "search"));

    // 2. Read status
    let status_bytes = router.read(SYNTHETIC_STATUS, &fs).await.unwrap();
    let status_str = String::from_utf8(status_bytes).unwrap();
    assert!(status_str.contains("SurrealFS Native FUSE Control Plane"));
    assert!(status_str.contains("martin"));

    // 3. Read whoami
    let whoami_bytes = router.read(SYNTHETIC_WHOAMI, &fs).await.unwrap();
    let whoami_str = String::from_utf8(whoami_bytes).unwrap();
    assert!(whoami_str.contains("caller: martin"));

    // 4. Read stats
    let stats_bytes = router.read(SYNTHETIC_STATS, &fs).await.unwrap();
    let stats_str = String::from_utf8(stats_bytes).unwrap();
    assert!(stats_str.contains("active_locks"));

    // 5. Test Swarm Locks via Synthetic Dir
    fs.acquire_lock("/docs/system.md", 120, "updating specs", "martin")
        .await
        .unwrap();

    let lock_entries = router.readdir(SYNTHETIC_LOCKS_DIR, &fs).await.unwrap();
    assert_eq!(lock_entries.len(), 1);
    assert_eq!(lock_entries[0].1, "docs_system.md");

    // Lookup lock file by name
    let lock_ino = router
        .lookup(SYNTHETIC_LOCKS_DIR, "docs_system.md")
        .unwrap();
    let lock_json = String::from_utf8(router.read(lock_ino, &fs).await.unwrap()).unwrap();
    assert!(lock_json.contains("updating specs"));
    assert!(lock_json.contains("martin"));

    // Release lock via synthetic unlink (rm .surrealfs/locks/docs_system.md)
    router.unlink(lock_ino, &fs).await.unwrap();
    let locks_after = fs.list_locks().await.unwrap();
    assert!(locks_after.is_empty());

    // 6. Test Search via Dynamic Inode Lookup
    let search_ino = router
        .lookup(SYNTHETIC_SEARCH_DIR, "knn/Architecture")
        .unwrap();
    let search_json = String::from_utf8(router.read(search_ino, &fs).await.unwrap()).unwrap();
    assert!(search_json.contains("/docs/system.md"));

    // 7. Test Branches current switch
    let branch_ino = router.lookup(SYNTHETIC_BRANCHES_DIR, "current").unwrap();
    assert_eq!(branch_ino, SYNTHETIC_BRANCH_CURRENT);
    let curr = String::from_utf8(router.read(branch_ino, &fs).await.unwrap()).unwrap();
    assert_eq!(curr.trim(), "main");

    router.write(branch_ino, b"agent-patch-42\n").await.unwrap();
    let switched = String::from_utf8(router.read(branch_ino, &fs).await.unwrap()).unwrap();
    assert_eq!(switched.trim(), "agent-patch-42");
}

#[tokio::test]
async fn test_xattr_engine_operations() {
    let fs = common::create_test_fs().await;

    let path = "/notes/todo.md";
    let entry = fs
        .write_text(path, "- [ ] test surrealfs xattr engine", None)
        .await
        .unwrap();

    let mut custom_attrs = HashMap::new();

    // 1. Read xattrs
    let list = XattrEngine::list_xattrs(&entry, None);
    assert!(list.contains(&XATTR_OWNER.to_string()));
    assert!(list.contains(&XATTR_GENERATION.to_string()));
    assert!(!list.contains(&XATTR_ACTION.to_string()));
    assert!(!list.contains(&XATTR_LOCK.to_string()));

    let owner = XattrEngine::get_xattr(&entry, XATTR_OWNER, None, &custom_attrs)
        .unwrap()
        .unwrap();
    assert_eq!(String::from_utf8(owner).unwrap(), "root");

    let gen = XattrEngine::get_xattr(&entry, XATTR_GENERATION, None, &custom_attrs)
        .unwrap()
        .unwrap();
    assert_eq!(String::from_utf8(gen).unwrap(), "1");

    // 2. Reject modifying read-only attributes
    let err = XattrEngine::set_xattr(&fs, path, XATTR_OWNER, b"hacker", "root", &mut custom_attrs)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        surrealfs_core::SurrealFsError::PermissionDenied(_)
    ));

    // 3. User / Agent custom metadata
    XattrEngine::set_xattr(
        &fs,
        path,
        "user.agent.task_id",
        b"TASK-9901",
        "root",
        &mut custom_attrs,
    )
    .await
    .unwrap();

    let custom_val = XattrEngine::get_xattr(&entry, "user.agent.task_id", None, &custom_attrs)
        .unwrap()
        .unwrap();
    assert_eq!(String::from_utf8(custom_val).unwrap(), "TASK-9901");

    // 4. Write-only action: acquire lock via xattr
    XattrEngine::set_xattr(
        &fs,
        path,
        XATTR_LOCK,
        b"lease:120s:editing-xattr",
        "root",
        &mut custom_attrs,
    )
    .await
    .unwrap();

    let locks = fs.list_locks().await.unwrap();
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].holder, "root");
    assert_eq!(locks[0].reason, Some("editing-xattr".into()));

    // Verify locked_by appears in list_xattrs when active
    let list_with_lock = XattrEngine::list_xattrs(&entry, Some(&locks[0]));
    assert!(list_with_lock.contains(&XATTR_LOCKED_BY.to_string()));

    let locked_by_val =
        XattrEngine::get_xattr(&entry, XATTR_LOCKED_BY, Some(&locks[0]), &custom_attrs)
            .unwrap()
            .unwrap();
    assert!(String::from_utf8(locked_by_val).unwrap().contains("root"));
}
