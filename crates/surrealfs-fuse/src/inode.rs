use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

pub const ROOT_INODE: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordKey {
    pub table: String,
    pub id: String,
}

impl RecordKey {
    pub fn new(table: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            id: id.into(),
        }
    }

    pub fn to_record_id_string(&self) -> String {
        format!("{}:{}", self.table, self.id)
    }
}

pub struct InodeTable {
    next_inode: AtomicU64,
    inode_to_key: RwLock<HashMap<u64, RecordKey>>,
    key_to_inode: RwLock<HashMap<RecordKey, u64>>,
}

impl InodeTable {
    pub fn new() -> Self {
        let mut inode_to_key = HashMap::new();
        let mut key_to_inode = HashMap::new();

        let root_key = RecordKey::new("file", "root");
        inode_to_key.insert(ROOT_INODE, root_key.clone());
        key_to_inode.insert(root_key, ROOT_INODE);

        Self {
            next_inode: AtomicU64::new(2),
            inode_to_key: RwLock::new(inode_to_key),
            key_to_inode: RwLock::new(key_to_inode),
        }
    }

    pub fn get_or_allocate(&self, table: &str, id: &str) -> u64 {
        let key = RecordKey::new(table, id);

        // Fast path: read lock
        {
            let map = self.key_to_inode.read().unwrap();
            if let Some(&ino) = map.get(&key) {
                return ino;
            }
        }

        // Write path
        let mut key_map = self.key_to_inode.write().unwrap();
        if let Some(&ino) = key_map.get(&key) {
            return ino;
        }

        let ino = self.next_inode.fetch_add(1, Ordering::SeqCst);
        let mut ino_map = self.inode_to_key.write().unwrap();

        key_map.insert(key.clone(), ino);
        ino_map.insert(ino, key);

        ino
    }

    pub fn get_key(&self, inode: u64) -> Option<RecordKey> {
        let map = self.inode_to_key.read().unwrap();
        map.get(&inode).cloned()
    }

    pub fn get_inode(&self, table: &str, id: &str) -> Option<u64> {
        let key = RecordKey::new(table, id);
        let map = self.key_to_inode.read().unwrap();
        map.get(&key).copied()
    }
}

impl Default for InodeTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_inode_is_one() {
        let table = InodeTable::new();
        assert_eq!(table.get_or_allocate("file", "root"), ROOT_INODE);
        let key = table.get_key(ROOT_INODE).unwrap();
        assert_eq!(key.table, "file");
        assert_eq!(key.id, "root");
    }

    #[test]
    fn test_allocation_is_idempotent() {
        let table = InodeTable::new();
        let ino1 = table.get_or_allocate("file", "note1");
        let ino2 = table.get_or_allocate("file", "note1");
        assert_eq!(ino1, ino2);
        assert!(ino1 >= 2);

        let key = table.get_key(ino1).unwrap();
        assert_eq!(key.id, "note1");
    }
}
