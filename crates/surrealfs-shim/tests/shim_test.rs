mod common;

use common::create_test_fs;
use surrealfs_shim::{VirtualFs, WasiVirtualFs, MIN_VIRTUAL_FD};

#[tokio::test]
async fn test_virtual_fs_open_write_read_close() {
    let fs = create_test_fs().await;
    let vfs = VirtualFs::new(fs.clone(), "/surrealfs");

    assert!(vfs.is_virtual_path("/surrealfs/notes/todo.txt"));
    assert!(!vfs.is_virtual_path("/etc/passwd"));
    assert_eq!(
        vfs.to_virtual_path("/surrealfs/notes/todo.txt"),
        Some("/notes/todo.txt".to_string())
    );

    // 1. Open with O_CREAT | O_WRONLY
    let fd = vfs
        .open(
            "/surrealfs/notes/todo.txt",
            libc::O_CREAT | libc::O_WRONLY,
            0o644,
        )
        .await
        .expect("Failed to open virtual file for create");
    assert!(fd >= MIN_VIRTUAL_FD);

    // 2. Write data
    let written = vfs
        .write(fd, b"Buy groceries\nCall agent\n")
        .expect("write failed");
    assert_eq!(written, 25);

    // 3. Close file (flushes to database)
    vfs.close(fd).await.expect("close failed");

    // 4. Verify in core SurrealFs
    let raw = fs
        .read_text("/notes/todo.txt")
        .await
        .expect("read_text from db failed");
    assert_eq!(raw, "Buy groceries\nCall agent\n");

    // 5. Open for reading
    let fd_read = vfs
        .open("/surrealfs/notes/todo.txt", libc::O_RDONLY, 0)
        .await
        .expect("Failed to open for read");

    let data = vfs.read(fd_read, 100).expect("read failed");
    assert_eq!(data, b"Buy groceries\nCall agent\n");

    vfs.close(fd_read).await.expect("close failed");
}

#[tokio::test]
async fn test_virtual_fs_seek_and_pread_pwrite() {
    let fs = create_test_fs().await;
    let vfs = VirtualFs::new(fs.clone(), "/mnt/surrealfs");

    // Create file
    let fd = vfs
        .open(
            "/mnt/surrealfs/data.bin",
            libc::O_CREAT | libc::O_RDWR,
            0o644,
        )
        .await
        .unwrap();

    vfs.write(fd, b"0123456789").unwrap();

    // Seek to 4
    let pos = vfs.lseek(fd, 4, libc::SEEK_SET).unwrap();
    assert_eq!(pos, 4);

    let part = vfs.read(fd, 3).unwrap();
    assert_eq!(part, b"456");

    // pread at offset 2 (should not change cursor)
    let pread_data = vfs.pread(fd, 4, 2).unwrap();
    assert_eq!(pread_data, b"2345");

    // cursor should still be at 7
    let next_data = vfs.read(fd, 3).unwrap();
    assert_eq!(next_data, b"789");

    // pwrite at offset 0
    vfs.pwrite(fd, b"ABCD", 0).unwrap();

    vfs.close(fd).await.unwrap();

    let final_content = fs.read_text("/data.bin").await.unwrap();
    assert_eq!(final_content, "ABCD456789");
}

#[tokio::test]
async fn test_virtual_fs_dir_operations() {
    let fs = create_test_fs().await;
    let vfs = VirtualFs::new(fs.clone(), "/surrealfs");

    vfs.mkdir("/surrealfs/projects").await.unwrap();

    let fd1 = vfs
        .open(
            "/surrealfs/projects/a.txt",
            libc::O_CREAT | libc::O_WRONLY,
            0o644,
        )
        .await
        .unwrap();
    vfs.write(fd1, b"proj a").unwrap();
    vfs.close(fd1).await.unwrap();

    let fd2 = vfs
        .open(
            "/surrealfs/projects/b.txt",
            libc::O_CREAT | libc::O_WRONLY,
            0o644,
        )
        .await
        .unwrap();
    vfs.write(fd2, b"proj b").unwrap();
    vfs.close(fd2).await.unwrap();

    // List directory
    let dir_id = vfs.opendir("/surrealfs/projects").await.unwrap();
    let mut names = Vec::new();
    while let Some(entry) = vfs.readdir(dir_id).unwrap() {
        names.push(entry.filename);
    }
    vfs.closedir(dir_id).unwrap();

    assert!(names.contains(&"a.txt".to_string()));
    assert!(names.contains(&"b.txt".to_string()));

    // Rename
    vfs.rename(
        "/surrealfs/projects/a.txt",
        "/surrealfs/projects/a_renamed.txt",
    )
    .await
    .unwrap();
    assert!(fs.exists("/projects/a_renamed.txt").await.unwrap());
    assert!(!fs.exists("/projects/a.txt").await.unwrap());

    // Unlink
    vfs.unlink("/surrealfs/projects/b.txt").await.unwrap();
    assert!(!fs.exists("/projects/b.txt").await.unwrap());
}

#[tokio::test]
async fn test_wasi_virtual_fs_adapter() {
    let fs = create_test_fs().await;
    let wasi_vfs = WasiVirtualFs::new(fs.clone(), "/surrealfs");

    assert_eq!(wasi_vfs.mount_prefix(), "/surrealfs");

    // Open & write
    let fd = wasi_vfs
        .open("/surrealfs/wasi_doc.md", false, true, true, false)
        .await
        .unwrap();
    wasi_vfs.write(fd, b"# WASI SurrealFS\n").unwrap();
    wasi_vfs.close(fd).await.unwrap();

    // Stat
    let meta = wasi_vfs.stat("/surrealfs/wasi_doc.md").await.unwrap();
    assert_eq!(meta.size, 17);
    assert_eq!(
        meta.descriptor_type,
        surrealfs_shim::WasiDescriptorType::RegularFile
    );

    // Read dir
    let entries = wasi_vfs.read_dir("/surrealfs").await.unwrap();
    assert!(entries.iter().any(|e| e.filename == "wasi_doc.md"));
}
