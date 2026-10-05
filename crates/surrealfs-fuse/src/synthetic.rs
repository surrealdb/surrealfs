use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use surrealfs_core::{SurrealFs, SurrealFsError};

pub const SYNTHETIC_INODE_BASE: u64 = 0x8000_0000_0000_0000;
pub const SYNTHETIC_ROOT: u64 = SYNTHETIC_INODE_BASE + 1;
pub const SYNTHETIC_STATUS: u64 = SYNTHETIC_INODE_BASE + 2;
pub const SYNTHETIC_STATS: u64 = SYNTHETIC_INODE_BASE + 3;
pub const SYNTHETIC_WHOAMI: u64 = SYNTHETIC_INODE_BASE + 4;
pub const SYNTHETIC_LOCKS_DIR: u64 = SYNTHETIC_INODE_BASE + 5;
pub const SYNTHETIC_SEARCH_DIR: u64 = SYNTHETIC_INODE_BASE + 6;
pub const SYNTHETIC_BRANCHES_DIR: u64 = SYNTHETIC_INODE_BASE + 7;
pub const SYNTHETIC_BRANCH_CURRENT: u64 = SYNTHETIC_INODE_BASE + 8;
pub const SYNTHETIC_HISTORY_DIR: u64 = SYNTHETIC_INODE_BASE + 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntheticNode {
    Root,
    Status,
    Stats,
    Whoami,
    LocksDir,
    LockFile { path: String },
    SearchDir,
    SearchQuery { query: String },
    BranchesDir,
    BranchCurrent,
    HistoryDir,
    HistoryFile { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    pub name: String,
    pub version: String,
    pub status: String,
    pub connected_url: String,
    pub caller: String,
    pub active_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub active_locks: usize,
    pub status: String,
}

pub struct SyntheticRouter {
    next_dynamic_inode: AtomicU64,
    inode_to_node: RwLock<HashMap<u64, SyntheticNode>>,
    dynamic_name_to_inode: RwLock<HashMap<String, u64>>,
    current_branch: RwLock<String>,
    connected_url: String,
    caller: String,
}

impl SyntheticRouter {
    pub fn new(connected_url: String, caller: String, initial_branch: String) -> Self {
        let mut map = HashMap::new();
        map.insert(SYNTHETIC_ROOT, SyntheticNode::Root);
        map.insert(SYNTHETIC_STATUS, SyntheticNode::Status);
        map.insert(SYNTHETIC_STATS, SyntheticNode::Stats);
        map.insert(SYNTHETIC_WHOAMI, SyntheticNode::Whoami);
        map.insert(SYNTHETIC_LOCKS_DIR, SyntheticNode::LocksDir);
        map.insert(SYNTHETIC_SEARCH_DIR, SyntheticNode::SearchDir);
        map.insert(SYNTHETIC_BRANCHES_DIR, SyntheticNode::BranchesDir);
        map.insert(SYNTHETIC_BRANCH_CURRENT, SyntheticNode::BranchCurrent);
        map.insert(SYNTHETIC_HISTORY_DIR, SyntheticNode::HistoryDir);

        Self {
            next_dynamic_inode: AtomicU64::new(SYNTHETIC_INODE_BASE + 1000),
            inode_to_node: RwLock::new(map),
            dynamic_name_to_inode: RwLock::new(HashMap::new()),
            current_branch: RwLock::new(initial_branch),
            connected_url,
            caller,
        }
    }

    pub fn is_synthetic_inode(ino: u64) -> bool {
        (ino & SYNTHETIC_INODE_BASE) != 0
    }

    pub fn lookup(&self, parent_ino: u64, name: &str) -> Option<u64> {
        if parent_ino == 1 && name == ".surrealfs" {
            return Some(SYNTHETIC_ROOT);
        }

        match parent_ino {
            SYNTHETIC_ROOT => match name {
                "status" => Some(SYNTHETIC_STATUS),
                "stats" => Some(SYNTHETIC_STATS),
                "whoami" => Some(SYNTHETIC_WHOAMI),
                "locks" => Some(SYNTHETIC_LOCKS_DIR),
                "search" => Some(SYNTHETIC_SEARCH_DIR),
                "branches" => Some(SYNTHETIC_BRANCHES_DIR),
                "history" => Some(SYNTHETIC_HISTORY_DIR),
                _ => None,
            },
            SYNTHETIC_BRANCHES_DIR => match name {
                "current" => Some(SYNTHETIC_BRANCH_CURRENT),
                _ => None,
            },
            SYNTHETIC_LOCKS_DIR => {
                let key = format!("lock:{}", name);
                let mut name_map = self.dynamic_name_to_inode.write().unwrap();
                if let Some(&ino) = name_map.get(&key) {
                    Some(ino)
                } else {
                    let ino = self.next_dynamic_inode.fetch_add(1, Ordering::SeqCst);
                    name_map.insert(key, ino);

                    let mut node_map = self.inode_to_node.write().unwrap();
                    let restored_path = if name.starts_with('/') {
                        name.to_string()
                    } else {
                        format!("/{}", name.replace('_', "/"))
                    };
                    node_map.insert(
                        ino,
                        SyntheticNode::LockFile {
                            path: restored_path,
                        },
                    );
                    Some(ino)
                }
            }
            SYNTHETIC_SEARCH_DIR => {
                let key = format!("search:{}", name);
                let mut name_map = self.dynamic_name_to_inode.write().unwrap();
                if let Some(&ino) = name_map.get(&key) {
                    Some(ino)
                } else {
                    let ino = self.next_dynamic_inode.fetch_add(1, Ordering::SeqCst);
                    name_map.insert(key, ino);

                    let mut node_map = self.inode_to_node.write().unwrap();
                    node_map.insert(
                        ino,
                        SyntheticNode::SearchQuery {
                            query: name.to_string(),
                        },
                    );
                    Some(ino)
                }
            }
            SYNTHETIC_HISTORY_DIR => {
                let key = format!("history:{}", name);
                let mut name_map = self.dynamic_name_to_inode.write().unwrap();
                if let Some(&ino) = name_map.get(&key) {
                    Some(ino)
                } else {
                    let ino = self.next_dynamic_inode.fetch_add(1, Ordering::SeqCst);
                    name_map.insert(key, ino);

                    let mut node_map = self.inode_to_node.write().unwrap();
                    let restored_path = if name.starts_with('/') {
                        name.to_string()
                    } else {
                        format!("/{}", name.replace('_', "/"))
                    };
                    node_map.insert(
                        ino,
                        SyntheticNode::HistoryFile {
                            path: restored_path,
                        },
                    );
                    Some(ino)
                }
            }
            _ => None,
        }
    }

    pub fn is_dir(&self, ino: u64) -> bool {
        let map = self.inode_to_node.read().unwrap();
        matches!(
            map.get(&ino),
            Some(
                SyntheticNode::Root
                    | SyntheticNode::LocksDir
                    | SyntheticNode::SearchDir
                    | SyntheticNode::BranchesDir
                    | SyntheticNode::HistoryDir,
            )
        )
    }

    pub async fn readdir(
        &self,
        ino: u64,
        fs: &SurrealFs,
    ) -> Result<Vec<(u64, String, bool)>, SurrealFsError> {
        match ino {
            SYNTHETIC_ROOT => Ok(vec![
                (SYNTHETIC_STATUS, "status".to_string(), false),
                (SYNTHETIC_STATS, "stats".to_string(), false),
                (SYNTHETIC_WHOAMI, "whoami".to_string(), false),
                (SYNTHETIC_LOCKS_DIR, "locks".to_string(), true),
                (SYNTHETIC_SEARCH_DIR, "search".to_string(), true),
                (SYNTHETIC_BRANCHES_DIR, "branches".to_string(), true),
                (SYNTHETIC_HISTORY_DIR, "history".to_string(), true),
            ]),
            SYNTHETIC_BRANCHES_DIR => Ok(vec![(
                SYNTHETIC_BRANCH_CURRENT,
                "current".to_string(),
                false,
            )]),
            SYNTHETIC_LOCKS_DIR => {
                let locks = fs.list_locks().await.unwrap_or_default();
                let mut entries = Vec::new();
                for l in locks {
                    let sanitized = l.path.trim_start_matches('/').replace('/', "_");
                    if let Some(child_ino) = self.lookup(SYNTHETIC_LOCKS_DIR, &sanitized) {
                        entries.push((child_ino, sanitized, false));
                    }
                }
                Ok(entries)
            }
            _ => Ok(Vec::new()),
        }
    }

    pub async fn read(&self, ino: u64, fs: &SurrealFs) -> Result<Vec<u8>, SurrealFsError> {
        let node = {
            let map = self.inode_to_node.read().unwrap();
            map.get(&ino).cloned()
        };

        match node {
            Some(SyntheticNode::Status) => {
                let branch = self.current_branch.read().unwrap().clone();
                let status = SystemStatus {
                    name: "SurrealFS Native FUSE Control Plane".to_string(),
                    version: "0.2.0".to_string(),
                    status: "connected".to_string(),
                    connected_url: self.connected_url.clone(),
                    caller: self.caller.clone(),
                    active_branch: branch,
                };
                let json = serde_json::to_string_pretty(&status).unwrap();
                Ok(format!("{}\n", json).into_bytes())
            }
            Some(SyntheticNode::Stats) => {
                let locks = fs.list_locks().await.unwrap_or_default();
                let stats = SystemStats {
                    active_locks: locks.len(),
                    status: "healthy".to_string(),
                };
                let json = serde_json::to_string_pretty(&stats).unwrap();
                Ok(format!("{}\n", json).into_bytes())
            }
            Some(SyntheticNode::Whoami) => {
                let info = format!(
                    "caller: {}\nconnected: {}\n",
                    self.caller, self.connected_url
                );
                Ok(info.into_bytes())
            }
            Some(SyntheticNode::BranchCurrent) => {
                let branch = self.current_branch.read().unwrap().clone();
                Ok(format!("{}\n", branch).into_bytes())
            }
            Some(SyntheticNode::LockFile { path }) => {
                let locks = fs.list_locks().await.unwrap_or_default();
                if let Some(lock) = locks.into_iter().find(|l| l.path == path) {
                    let json = serde_json::to_string_pretty(&lock).unwrap();
                    Ok(format!("{}\n", json).into_bytes())
                } else {
                    Err(SurrealFsError::NotFound(format!("No lock for {}", path)))
                }
            }
            Some(SyntheticNode::SearchQuery { query }) => {
                let clean_query = query.trim_start_matches("knn/").trim_start_matches("q=");
                let matches = fs
                    .grep(clean_query, Some("/"), true)
                    .await
                    .unwrap_or_default();
                let json = serde_json::to_string_pretty(&matches).unwrap();
                Ok(format!("{}\n", json).into_bytes())
            }
            Some(SyntheticNode::HistoryFile { path }) => {
                let history = fs.history(&path, Some(20)).await.unwrap_or_default();
                let json = serde_json::to_string_pretty(&history).unwrap();
                Ok(format!("{}\n", json).into_bytes())
            }
            _ => Err(SurrealFsError::NotFound("Synthetic node not found".into())),
        }
    }

    pub async fn write(&self, ino: u64, data: &[u8]) -> Result<(), SurrealFsError> {
        let node = {
            let map = self.inode_to_node.read().unwrap();
            map.get(&ino).cloned()
        };

        match node {
            Some(SyntheticNode::BranchCurrent) => {
                let new_branch = std::str::from_utf8(data)
                    .map_err(|e| SurrealFsError::InvalidPath(e.to_string()))?
                    .trim()
                    .to_string();
                if new_branch.is_empty() {
                    return Err(SurrealFsError::InvalidPath("Branch cannot be empty".into()));
                }
                let mut b = self.current_branch.write().unwrap();
                *b = new_branch;
                Ok(())
            }
            _ => Err(SurrealFsError::PermissionDenied(
                "Synthetic node is read-only".into(),
            )),
        }
    }

    pub async fn unlink(&self, ino: u64, fs: &SurrealFs) -> Result<(), SurrealFsError> {
        let node = {
            let map = self.inode_to_node.read().unwrap();
            map.get(&ino).cloned()
        };

        match node {
            Some(SyntheticNode::LockFile { path }) => {
                fs.release_lock(&path, &self.caller).await?;
                Ok(())
            }
            _ => Err(SurrealFsError::PermissionDenied(
                "Cannot delete synthetic node".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthetic_inode_recognition() {
        assert!(SyntheticRouter::is_synthetic_inode(SYNTHETIC_ROOT));
        assert!(SyntheticRouter::is_synthetic_inode(SYNTHETIC_STATUS));
        assert!(!SyntheticRouter::is_synthetic_inode(1));
        assert!(!SyntheticRouter::is_synthetic_inode(42));
    }

    #[test]
    fn test_synthetic_lookup_hierarchy() {
        let router =
            SyntheticRouter::new("ws://localhost:8000".into(), "root".into(), "main".into());

        let root = router.lookup(1, ".surrealfs").unwrap();
        assert_eq!(root, SYNTHETIC_ROOT);

        let status = router.lookup(SYNTHETIC_ROOT, "status").unwrap();
        assert_eq!(status, SYNTHETIC_STATUS);

        let locks = router.lookup(SYNTHETIC_ROOT, "locks").unwrap();
        assert_eq!(locks, SYNTHETIC_LOCKS_DIR);

        let branch_curr = router.lookup(SYNTHETIC_BRANCHES_DIR, "current").unwrap();
        assert_eq!(branch_curr, SYNTHETIC_BRANCH_CURRENT);
    }

    #[tokio::test]
    async fn test_branch_switch_via_synthetic_write() {
        let router =
            SyntheticRouter::new("ws://localhost:8000".into(), "root".into(), "main".into());

        router
            .write(SYNTHETIC_BRANCH_CURRENT, b"agent-feature-x\n")
            .await
            .unwrap();

        assert_eq!(*router.current_branch.read().unwrap(), "agent-feature-x");
    }
}
