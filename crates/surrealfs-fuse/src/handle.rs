use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct FileHandle {
    pub fh: u64,
    pub inode: u64,
    pub path: String,
    pub base_generation: Option<u64>,
    pub write_buffer: Vec<u8>,
    pub is_dirty: bool,
}

impl FileHandle {
    pub fn new(fh: u64, inode: u64, path: String, base_generation: Option<u64>) -> Self {
        Self {
            fh,
            inode,
            path,
            base_generation,
            write_buffer: Vec::new(),
            is_dirty: false,
        }
    }

    pub fn write_chunk(&mut self, offset: usize, data: &[u8]) {
        let req_len = offset + data.len();
        if self.write_buffer.len() < req_len {
            self.write_buffer.resize(req_len, 0);
        }
        self.write_buffer[offset..req_len].copy_from_slice(data);
        self.is_dirty = true;
    }
}

pub struct HandleTable {
    next_fh: AtomicU64,
    handles: RwLock<HashMap<u64, FileHandle>>,
}

impl HandleTable {
    pub fn new() -> Self {
        Self {
            next_fh: AtomicU64::new(1),
            handles: RwLock::new(HashMap::new()),
        }
    }

    pub fn allocate(&self, inode: u64, path: String, base_generation: Option<u64>) -> u64 {
        let fh = self.next_fh.fetch_add(1, Ordering::SeqCst);
        let handle = FileHandle::new(fh, inode, path, base_generation);
        let mut map = self.handles.write().unwrap();
        map.insert(fh, handle);
        fh
    }

    pub fn get(&self, fh: u64) -> Option<FileHandle> {
        let map = self.handles.read().unwrap();
        map.get(&fh).cloned()
    }

    pub fn write(&self, fh: u64, offset: usize, data: &[u8]) -> bool {
        let mut map = self.handles.write().unwrap();
        if let Some(handle) = map.get_mut(&fh) {
            handle.write_chunk(offset, data);
            true
        } else {
            false
        }
    }

    pub fn remove(&self, fh: u64) -> Option<FileHandle> {
        let mut map = self.handles.write().unwrap();
        map.remove(&fh)
    }
}

impl Default for HandleTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_allocation_and_buffering() {
        let table = HandleTable::new();
        let fh = table.allocate(2, "/test.txt".into(), Some(1));
        assert!(fh >= 1);

        assert!(table.write(fh, 0, b"Hello"));
        assert!(table.write(fh, 5, b" World"));

        let handle = table.get(fh).unwrap();
        assert_eq!(&handle.write_buffer, b"Hello World");
        assert!(handle.is_dirty);
        assert_eq!(handle.base_generation, Some(1));

        let removed = table.remove(fh).unwrap();
        assert_eq!(removed.fh, fh);
        assert!(table.get(fh).is_none());
    }
}
