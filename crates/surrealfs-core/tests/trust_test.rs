mod common;

use common::create_test_fs;
use surrealfs_core::errors::SurrealFsError;

#[tokio::test]
async fn test_groups_and_membership() {
    let fs = create_test_fs().await;

    // Create group
    let grp = fs
        .create_group("engineering", Some("Engineering team"))
        .await
        .expect("create_group");
    assert_eq!(grp.name, "engineering");
    assert_eq!(grp.description.as_deref(), Some("Engineering team"));

    // Add members
    fs.add_group_member("engineering", "alice")
        .await
        .expect("add alice");
    fs.add_group_member("engineering", "bob")
        .await
        .expect("add bob");

    // List members
    let members = fs
        .list_group_members("engineering")
        .await
        .expect("list_group_members");
    assert_eq!(members.len(), 2);
    let user_ids: Vec<&str> = members.iter().map(|m| m.user_id.as_str()).collect();
    assert!(user_ids.contains(&"alice"));
    assert!(user_ids.contains(&"bob"));

    // Get user groups
    let alice_groups = fs.get_user_groups("alice").await.expect("get_user_groups");
    assert_eq!(alice_groups, vec!["engineering".to_string()]);

    // Remove member
    fs.remove_group_member("engineering", "bob")
        .await
        .expect("remove bob");
    let members_after = fs
        .list_group_members("engineering")
        .await
        .expect("list after");
    assert_eq!(members_after.len(), 1);
    assert_eq!(members_after[0].user_id, "alice");
}

#[tokio::test]
async fn test_chgrp_and_file_group() {
    let fs = create_test_fs().await;

    fs.create_group("secops", None).await.expect("create group");
    fs.write_text("/audit.log", "audit line 1", None)
        .await
        .expect("write_text");

    let entry = fs.chgrp("/audit.log", Some("secops")).await.expect("chgrp");
    assert_eq!(entry.group.as_deref(), Some("secops"));

    let stat = fs.stat("/audit.log").await.expect("stat").expect("entry");
    assert_eq!(stat.group.as_deref(), Some("secops"));
}

#[tokio::test]
async fn test_group_permission_gating() {
    let root_fs = create_test_fs().await;

    // Create group "devs" and add "bob" to it
    root_fs
        .create_group("devs", None)
        .await
        .expect("create group");
    root_fs
        .add_group_member("devs", "bob")
        .await
        .expect("add bob");

    let alice_fs = root_fs.with_caller("alice");
    let bob_fs = root_fs.with_caller("bob");
    let eve_fs = root_fs.with_caller("eve");

    // Alice creates /project directory
    alice_fs
        .mkdir("/project", false)
        .await
        .expect("mkdir /project");
    // Assign group "devs" and set mode 0o750 (rwxr-x---) on /project
    root_fs
        .chgrp("/project", Some("devs"))
        .await
        .expect("chgrp devs");
    root_fs.chmod("/project", 0o750).await.expect("chmod 0750");

    // Alice writes a file inside /project
    alice_fs
        .write_text("/project/plan.md", "Secret Plan", None)
        .await
        .expect("write plan");
    root_fs
        .chgrp("/project/plan.md", Some("devs"))
        .await
        .expect("chgrp plan");
    root_fs
        .chmod("/project/plan.md", 0o640)
        .await
        .expect("chmod plan 0640");

    // Bob is in "devs" -> can read
    let bob_read = bob_fs.read_text("/project/plan.md").await;
    assert!(
        bob_read.is_ok(),
        "Bob should be allowed to read /project/plan.md: {:?}",
        bob_read.err()
    );
    assert_eq!(bob_read.unwrap(), "Secret Plan");

    // Eve is NOT in "devs" -> cannot read (permission denied)
    let eve_read = eve_fs.read_text("/project/plan.md").await;
    assert!(
        eve_read.is_err(),
        "Eve should NOT be allowed to read /project/plan.md"
    );
}

#[tokio::test]
async fn test_share_links_lifecycle() {
    let fs = create_test_fs().await;

    fs.write_text("/shared_doc.md", "# Public Doc", None)
        .await
        .expect("write_text");

    // Create share with 3600 seconds expiration
    let share = fs
        .create_share("/shared_doc.md", "ro", Some(3600))
        .await
        .expect("create_share");
    assert_eq!(share.path, "/shared_doc.md");
    assert_eq!(share.mode, "ro");
    assert!(!share.token.is_empty());
    assert!(!share.revoked);
    assert!(share.expires_at.is_some());

    // Resolve share
    let resolved = fs.resolve_share(&share.token).await.expect("resolve_share");
    assert!(resolved.is_some());
    let r = resolved.unwrap();
    assert_eq!(r.token, share.token);
    assert_eq!(r.path, "/shared_doc.md");

    // List shares
    let shares = fs.list_shares().await.expect("list_shares");
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].token, share.token);

    // Revoke share
    fs.revoke_share(&share.token).await.expect("revoke_share");
    let resolved_after = fs
        .resolve_share(&share.token)
        .await
        .expect("resolve after revoke");
    assert!(resolved_after.is_none());
}

#[tokio::test]
async fn test_legal_hold_blocks_deletion() {
    let fs = create_test_fs().await;

    fs.mkdir("/compliance", true).await.expect("mkdir");
    fs.write_text(
        "/compliance/financial_records.pdf",
        "irrevocable data",
        None,
    )
    .await
    .expect("write");

    // Initially not on legal hold
    assert!(!fs
        .is_legal_hold("/compliance/financial_records.pdf")
        .await
        .expect("is_legal_hold"));

    // Enable legal hold
    fs.set_legal_hold("/compliance/financial_records.pdf", true)
        .await
        .expect("set_legal_hold");
    assert!(fs
        .is_legal_hold("/compliance/financial_records.pdf")
        .await
        .expect("is_legal_hold"));

    // Attempt to delete file -> should fail with LegalHold error
    let rm_res = fs.rm("/compliance/financial_records.pdf", false).await;
    assert!(rm_res.is_err(), "Expected rm to fail due to legal hold");
    match rm_res.err().unwrap() {
        SurrealFsError::LegalHold(_) => {}
        other => panic!("Expected SurrealFsError::LegalHold, got: {:?}", other),
    }

    // Attempt recursive delete on parent folder -> should fail with LegalHold error
    let rm_dir_res = fs.rm("/compliance", true).await;
    assert!(
        rm_dir_res.is_err(),
        "Expected recursive rm to fail due to child legal hold"
    );
    match rm_dir_res.err().unwrap() {
        SurrealFsError::LegalHold(_) => {}
        other => panic!("Expected SurrealFsError::LegalHold, got: {:?}", other),
    }

    // Release legal hold
    fs.set_legal_hold("/compliance/financial_records.pdf", false)
        .await
        .expect("release hold");
    assert!(!fs
        .is_legal_hold("/compliance/financial_records.pdf")
        .await
        .expect("is_legal_hold"));

    // Now rm succeeds
    fs.rm("/compliance/financial_records.pdf", false)
        .await
        .expect("rm after release");
    fs.rm("/compliance", false).await.expect("rm dir");
}

#[tokio::test]
async fn test_webhooks_management() {
    let fs = create_test_fs().await;

    // Create webhook
    let wh = fs
        .create_webhook(
            "/events/*",
            &["create", "update"],
            "https://example.com/webhook",
            "supersecret123",
        )
        .await
        .expect("create_webhook");

    assert_eq!(wh.path_pattern, "/events/*");
    assert_eq!(wh.events, vec!["create", "update"]);
    assert_eq!(wh.url, "https://example.com/webhook");
    assert_eq!(wh.secret, "supersecret123");
    assert!(wh.active);

    // List webhooks
    let list = fs.list_webhooks().await.expect("list_webhooks");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].url, "https://example.com/webhook");

    // Delete webhook
    if let Some(id) = wh.id {
        let clean_id = id.strip_prefix("webhook:").unwrap_or(&id);
        fs.delete_webhook(clean_id).await.expect("delete_webhook");
        let list_after = fs.list_webhooks().await.expect("list_webhooks after");
        assert_eq!(list_after.len(), 0);
    }
}
