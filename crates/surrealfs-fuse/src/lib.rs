pub mod errno;
pub mod handle;
pub mod inode;
pub mod synthetic;
pub mod xattr;

pub use errno::*;
pub use handle::*;
pub use inode::*;
pub use synthetic::*;
pub use xattr::*;

use std::collections::HashMap;
use std::sync::RwLock;
use surrealfs_core::{FileEntry, SurrealFs, SurrealFsError};

/// High-level VFS adapter bridging `InodeTable`, `HandleTable`, `SyntheticRouter`,
/// and `XattrEngine` to `SurrealFs`.
pub struct FuseAdapter {
    pub fs: SurrealFs,
    pub inodes: InodeTable,
    pub handles: HandleTable,
    pub router: SyntheticRouter,
    custom_xattrs: RwLock<HashMap<String, HashMap<String, String>>>,
}

impl FuseAdapter {
    pub fn new(fs: SurrealFs, router: SyntheticRouter) -> Self {
        Self {
            fs,
            inodes: InodeTable::new(),
            handles: HandleTable::new(),
            router,
            custom_xattrs: RwLock::new(HashMap::new()),
        }
    }

    pub async fn lookup_path(&self, path: &str) -> Result<(u64, FileEntry), SurrealFsError> {
        let entry = self
            .fs
            .stat(path)
            .await?
            .ok_or_else(|| SurrealFsError::NotFound(path.to_string()))?;
        let ino = self.inodes.get_or_allocate("file", &entry.path);
        Ok((ino, entry))
    }

    pub async fn open(&self, path: &str) -> Result<u64, SurrealFsError> {
        let (ino, entry) = self.lookup_path(path).await?;
        let fh = self
            .handles
            .allocate(ino, entry.path.clone(), Some(entry.generation));
        Ok(fh)
    }

    pub async fn read(
        &self,
        path: &str,
        offset: usize,
        size: usize,
    ) -> Result<Vec<u8>, SurrealFsError> {
        let bytes = self.fs.read_bytes(path).await?;
        let start = offset.min(bytes.len());
        let end = (offset + size).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    pub async fn read_inode(
        &self,
        ino: u64,
        offset: usize,
        size: usize,
    ) -> Result<Vec<u8>, SurrealFsError> {
        if SyntheticRouter::is_synthetic_inode(ino) {
            let bytes = self.router.read(ino, &self.fs).await?;
            let start = offset.min(bytes.len());
            let end = (offset + size).min(bytes.len());
            return Ok(bytes[start..end].to_vec());
        }
        let key = self
            .inodes
            .get_key(ino)
            .ok_or_else(|| SurrealFsError::NotFound(format!("Inode {} not found", ino)))?;
        self.read(&key.id, offset, size).await
    }

    pub fn write(&self, fh: u64, offset: usize, data: &[u8]) -> Result<usize, SurrealFsError> {
        if self.handles.write(fh, offset, data) {
            Ok(data.len())
        } else {
            Err(SurrealFsError::NotFound(format!(
                "Invalid file handle {}",
                fh
            )))
        }
    }

    pub async fn release(&self, fh: u64) -> Result<(), SurrealFsError> {
        if let Some(handle) = self.handles.remove(fh) {
            if handle.is_dirty {
                if SyntheticRouter::is_synthetic_inode(handle.inode) {
                    self.router
                        .write(handle.inode, &handle.write_buffer)
                        .await?;
                } else {
                    self.fs
                        .write_bytes(&handle.path, &handle.write_buffer, handle.base_generation)
                        .await?;
                }
            }
        }
        Ok(())
    }

    pub async fn getxattr(
        &self,
        path: &str,
        name: &str,
    ) -> Result<Option<Vec<u8>>, SurrealFsError> {
        let entry = self
            .fs
            .stat(path)
            .await?
            .ok_or_else(|| SurrealFsError::NotFound(path.to_string()))?;
        let locks = self.fs.list_locks().await.unwrap_or_default();
        let active_lock = locks.iter().find(|l| l.path == entry.path);
        let map = self.custom_xattrs.read().unwrap();
        let empty = HashMap::new();
        let custom = map.get(&entry.path).unwrap_or(&empty);
        XattrEngine::get_xattr(&entry, name, active_lock, custom)
    }

    pub async fn setxattr(
        &self,
        path: &str,
        name: &str,
        value: &[u8],
    ) -> Result<(), SurrealFsError> {
        let mut custom = {
            let map = self.custom_xattrs.read().unwrap();
            map.get(path).cloned().unwrap_or_default()
        };
        XattrEngine::set_xattr(
            &self.fs,
            path,
            name,
            value,
            &self.router.caller,
            &mut custom,
        )
        .await?;
        let mut map = self.custom_xattrs.write().unwrap();
        map.insert(path.to_string(), custom);
        Ok(())
    }
}
