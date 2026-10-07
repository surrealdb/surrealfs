use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use surrealfs_core::{FileEntry, SurrealFs, SurrealFsError};

pub const MIN_VIRTUAL_FD: i32 = 1_000_000;

#[derive(Debug, Clone)]
pub struct OpenFile {
    pub path: String,
    pub flags: i32,
    pub cursor: u64,
    pub buffer: Vec<u8>,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct OpenDir {
    pub path: String,
    pub cursor: usize,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone)]
pub struct VirtualFs {
    fs: SurrealFs,
    mount_prefix: String,
    next_fd: Arc<AtomicI32>,
    files: Arc<RwLock<HashMap<i32, OpenFile>>>,
    next_dir_id: Arc<AtomicUsize>,
    dirs: Arc<RwLock<HashMap<usize, OpenDir>>>,
}

impl VirtualFs {
    pub fn new(fs: SurrealFs, mount_prefix: &str) -> Self {
        let prefix = if mount_prefix.ends_with('/') {
            mount_prefix.trim_end_matches('/').to_string()
        } else {
            mount_prefix.to_string()
        };
        Self {
            fs,
            mount_prefix: prefix,
            next_fd: Arc::new(AtomicI32::new(MIN_VIRTUAL_FD)),
            files: Arc::new(RwLock::new(HashMap::new())),
            next_dir_id: Arc::new(AtomicUsize::new(1)),
            dirs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn mount_prefix(&self) -> &str {
        &self.mount_prefix
    }

    pub fn is_virtual_path(&self, path: &str) -> bool {
        path == self.mount_prefix || path.starts_with(&format!("{}/", self.mount_prefix))
    }

    pub fn to_virtual_path(&self, raw_path: &str) -> Option<String> {
        if !self.is_virtual_path(raw_path) {
            return None;
        }
        let rel = raw_path[self.mount_prefix.len()..].trim_start_matches('/');
        if rel.is_empty() {
            Some("/".to_string())
        } else {
            Some(format!("/{}", rel))
        }
    }

    pub fn is_virtual_fd(&self, fd: i32) -> bool {
        fd >= MIN_VIRTUAL_FD
    }

    pub async fn open(
        &self,
        raw_path: &str,
        flags: i32,
        _mode: u32,
    ) -> Result<i32, SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;

        let is_create = (flags & libc::O_CREAT) != 0;
        let is_trunc = (flags & libc::O_TRUNC) != 0;
        let is_append = (flags & libc::O_APPEND) != 0;

        let stat = self.fs.stat(&rel).await?;
        let mut buffer = Vec::new();

        match stat {
            Some(entry) => {
                if entry.is_folder {
                    return Err(SurrealFsError::Other(format!("Is a directory: {}", rel)));
                }
                if is_create && (flags & libc::O_EXCL) != 0 {
                    return Err(SurrealFsError::AlreadyExists(rel));
                }
                if !is_trunc {
                    buffer = self.fs.read_bytes(&rel).await?;
                }
            }
            None => {
                if !is_create {
                    return Err(SurrealFsError::NotFound(rel));
                }
                // File will be created on save or immediately
                self.fs.write_bytes(&rel, &[], None).await?;
            }
        }

        let cursor = if is_append { buffer.len() as u64 } else { 0 };
        let fd = self.next_fd.fetch_add(1, Ordering::SeqCst);

        let open_file = OpenFile {
            path: rel,
            flags,
            cursor,
            buffer,
            dirty: is_trunc,
        };

        self.files.write().insert(fd, open_file);
        Ok(fd)
    }

    pub fn read(&self, fd: i32, count: usize) -> Result<Vec<u8>, SurrealFsError> {
        let mut files = self.files.write();
        let file = files
            .get_mut(&fd)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?;

        let cur = file.cursor as usize;
        if cur >= file.buffer.len() {
            return Ok(Vec::new());
        }

        let end = std::cmp::min(cur + count, file.buffer.len());
        let slice = file.buffer[cur..end].to_vec();
        file.cursor = end as u64;
        Ok(slice)
    }

    pub fn write(&self, fd: i32, data: &[u8]) -> Result<usize, SurrealFsError> {
        let mut files = self.files.write();
        let file = files
            .get_mut(&fd)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?;

        if (file.flags & libc::O_APPEND) != 0 {
            file.cursor = file.buffer.len() as u64;
        }

        let cur = file.cursor as usize;
        if cur + data.len() > file.buffer.len() {
            file.buffer.resize(cur + data.len(), 0);
        }

        file.buffer[cur..cur + data.len()].copy_from_slice(data);
        file.cursor = (cur + data.len()) as u64;
        file.dirty = true;
        Ok(data.len())
    }

    pub fn pread(&self, fd: i32, count: usize, offset: u64) -> Result<Vec<u8>, SurrealFsError> {
        let files = self.files.read();
        let file = files
            .get(&fd)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?;

        let off = offset as usize;
        if off >= file.buffer.len() {
            return Ok(Vec::new());
        }

        let end = std::cmp::min(off + count, file.buffer.len());
        Ok(file.buffer[off..end].to_vec())
    }

    pub fn pwrite(&self, fd: i32, data: &[u8], offset: u64) -> Result<usize, SurrealFsError> {
        let mut files = self.files.write();
        let file = files
            .get_mut(&fd)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?;

        let off = offset as usize;
        if off + data.len() > file.buffer.len() {
            file.buffer.resize(off + data.len(), 0);
        }

        file.buffer[off..off + data.len()].copy_from_slice(data);
        file.dirty = true;
        Ok(data.len())
    }

    pub fn lseek(&self, fd: i32, offset: i64, whence: i32) -> Result<u64, SurrealFsError> {
        let mut files = self.files.write();
        let file = files
            .get_mut(&fd)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?;

        let new_pos = match whence {
            libc::SEEK_SET => {
                if offset < 0 {
                    return Err(SurrealFsError::Other(
                        "Invalid argument for SEEK_SET".into(),
                    ));
                }
                offset as u64
            }
            libc::SEEK_CUR => {
                let cur = file.cursor as i64;
                let target = cur + offset;
                if target < 0 {
                    return Err(SurrealFsError::Other(
                        "Invalid argument for SEEK_CUR".into(),
                    ));
                }
                target as u64
            }
            libc::SEEK_END => {
                let len = file.buffer.len() as i64;
                let target = len + offset;
                if target < 0 {
                    return Err(SurrealFsError::Other(
                        "Invalid argument for SEEK_END".into(),
                    ));
                }
                target as u64
            }
            _ => return Err(SurrealFsError::Other(format!("Unknown whence: {}", whence))),
        };

        file.cursor = new_pos;
        Ok(new_pos)
    }

    pub async fn close(&self, fd: i32) -> Result<(), SurrealFsError> {
        let file = {
            let mut files = self.files.write();
            files
                .remove(&fd)
                .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))?
        };

        if file.dirty {
            self.fs.write_bytes(&file.path, &file.buffer, None).await?;
        }
        Ok(())
    }

    pub async fn stat(&self, raw_path: &str) -> Result<FileEntry, SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;
        self.fs
            .stat(&rel)
            .await?
            .ok_or(SurrealFsError::NotFound(rel))
    }

    pub fn fstat(&self, fd: i32) -> Result<OpenFile, SurrealFsError> {
        let files = self.files.read();
        files
            .get(&fd)
            .cloned()
            .ok_or_else(|| SurrealFsError::Other(format!("Bad file descriptor: {}", fd)))
    }

    pub async fn mkdir(&self, raw_path: &str) -> Result<(), SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;
        self.fs.mkdir(&rel, true).await?;
        Ok(())
    }

    pub async fn unlink(&self, raw_path: &str) -> Result<(), SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;
        self.fs.rm(&rel, false).await
    }

    pub async fn rmdir(&self, raw_path: &str) -> Result<(), SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;
        self.fs.rm(&rel, false).await
    }

    pub async fn rename(&self, src_raw: &str, dst_raw: &str) -> Result<(), SurrealFsError> {
        let src_rel = self.to_virtual_path(src_raw).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", src_raw))
        })?;
        let dst_rel = self.to_virtual_path(dst_raw).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", dst_raw))
        })?;
        self.fs.mv(&src_rel, &dst_rel).await?;
        Ok(())
    }

    pub async fn opendir(&self, raw_path: &str) -> Result<usize, SurrealFsError> {
        let rel = self.to_virtual_path(raw_path).ok_or_else(|| {
            SurrealFsError::InvalidPath(format!("Path outside mount prefix: {}", raw_path))
        })?;

        let entries = self.fs.ls(&rel).await?;
        let dir_id = self.next_dir_id.fetch_add(1, Ordering::SeqCst);
        let open_dir = OpenDir {
            path: rel,
            cursor: 0,
            entries,
        };

        self.dirs.write().insert(dir_id, open_dir);
        Ok(dir_id)
    }

    pub fn readdir(&self, dir_id: usize) -> Result<Option<FileEntry>, SurrealFsError> {
        let mut dirs = self.dirs.write();
        let dir = dirs
            .get_mut(&dir_id)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad directory handle: {}", dir_id)))?;

        if dir.cursor < dir.entries.len() {
            let entry = dir.entries[dir.cursor].clone();
            dir.cursor += 1;
            Ok(Some(entry))
        } else {
            Ok(None)
        }
    }

    pub fn closedir(&self, dir_id: usize) -> Result<(), SurrealFsError> {
        self.dirs
            .write()
            .remove(&dir_id)
            .ok_or_else(|| SurrealFsError::Other(format!("Bad directory handle: {}", dir_id)))?;
        Ok(())
    }
}
