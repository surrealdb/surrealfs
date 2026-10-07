use crate::virtual_fs::VirtualFs;
use surrealfs_core::{FileEntry, SurrealFs, SurrealFsError};

/// WASI Preview 2 File Descriptor type abstraction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasiDescriptorType {
    Directory,
    RegularFile,
    Unknown,
}

/// WASI filesystem metadata
#[derive(Debug, Clone)]
pub struct WasiMetadata {
    pub descriptor_type: WasiDescriptorType,
    pub size: u64,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub modified_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// WASI Virtual Filesystem Adapter wrapping SurrealFS VirtualFs
pub struct WasiVirtualFs {
    vfs: VirtualFs,
}

impl WasiVirtualFs {
    pub fn new(fs: SurrealFs, mount_prefix: &str) -> Self {
        Self {
            vfs: VirtualFs::new(fs, mount_prefix),
        }
    }

    pub fn mount_prefix(&self) -> &str {
        self.vfs.mount_prefix()
    }

    pub async fn open(
        &self,
        path: &str,
        read: bool,
        write: bool,
        create: bool,
        truncate: bool,
    ) -> Result<i32, SurrealFsError> {
        let mut flags = 0;
        if read && write {
            flags |= libc::O_RDWR;
        } else if write {
            flags |= libc::O_WRONLY;
        } else {
            flags |= libc::O_RDONLY;
        }

        if create {
            flags |= libc::O_CREAT;
        }
        if truncate {
            flags |= libc::O_TRUNC;
        }

        self.vfs.open(path, flags, 0o644).await
    }

    pub fn read(&self, fd: i32, count: usize) -> Result<Vec<u8>, SurrealFsError> {
        self.vfs.read(fd, count)
    }

    pub fn write(&self, fd: i32, data: &[u8]) -> Result<usize, SurrealFsError> {
        self.vfs.write(fd, data)
    }

    pub async fn close(&self, fd: i32) -> Result<(), SurrealFsError> {
        self.vfs.close(fd).await
    }

    pub async fn stat(&self, path: &str) -> Result<WasiMetadata, SurrealFsError> {
        let entry = self.vfs.stat(path).await?;
        Ok(WasiMetadata {
            descriptor_type: if entry.is_folder {
                WasiDescriptorType::Directory
            } else {
                WasiDescriptorType::RegularFile
            },
            size: entry.size as u64,
            created_at: entry.created_at,
            modified_at: entry.updated_at,
        })
    }

    pub async fn read_dir(&self, path: &str) -> Result<Vec<FileEntry>, SurrealFsError> {
        let dir_id = self.vfs.opendir(path).await?;
        let mut out = Vec::new();
        while let Some(entry) = self.vfs.readdir(dir_id)? {
            out.push(entry);
        }
        self.vfs.closedir(dir_id)?;
        Ok(out)
    }
}
