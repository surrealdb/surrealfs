use anyhow::{bail, Result};
use std::collections::{HashMap, HashSet};
use surrealdb::types::Value as SValue;
use surrealfs_core::SurrealFs;

#[derive(Debug, serde::Deserialize)]
struct FileNode {
    pub id: serde_json::Value,
    #[allow(dead_code)]
    pub path: Option<String>,
    pub parent: Option<serde_json::Value>,
    pub parent_key: Option<String>,
    #[allow(dead_code)]
    pub is_folder: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct LockNode {
    pub file: serde_json::Value,
    #[allow(dead_code)]
    pub path: Option<String>,
    #[allow(dead_code)]
    pub holder: String,
    #[allow(dead_code)]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

fn json_to_str(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub struct InvariantChecker;

impl InvariantChecker {
    /// Invariant 1: Tree Hierarchy Invariant
    /// - No cycles in parent links.
    /// - No orphaned nodes: every node whose parent_key != 'root' has a parent that exists.
    pub async fn check_tree_hierarchy(fs: &SurrealFs) -> Result<()> {
        let mut res = fs
            .client()
            .query("SELECT id, path, parent, parent_key, is_folder FROM file;")
            .await?;

        let raw_nodes: Vec<SValue> = res.take(0usize)?;
        let mut nodes: Vec<FileNode> = Vec::new();
        for v in raw_nodes {
            if let Ok(node) = serde_json::from_value(v.into_json_value()) {
                nodes.push(node);
            }
        }

        let mut id_set = HashSet::new();
        let mut parent_map = HashMap::new();

        for node in &nodes {
            let id_str = json_to_str(&node.id);
            id_set.insert(id_str.clone());
            if let Some(ref p) = node.parent {
                let p_str = json_to_str(p);
                parent_map.insert(id_str, p_str);
            }
        }

        // Check for orphans
        for node in &nodes {
            if let Some(ref pk) = node.parent_key {
                if pk != "root" && !id_set.contains(pk) {
                    bail!(
                        "Hierarchy Invariant violated: orphan node {:?} points to non-existent parent_key {}",
                        node.id,
                        pk
                    );
                }
            }
        }

        // Check for cycles
        for start in &id_set {
            let mut visited = HashSet::new();
            let mut curr = start;
            while let Some(parent) = parent_map.get(curr) {
                if visited.contains(parent) {
                    bail!(
                        "Hierarchy Invariant violated: cycle detected at node {}",
                        parent
                    );
                }
                visited.insert(curr);
                curr = parent;
            }
        }

        Ok(())
    }

    /// Invariant 2: Lock Exclusivity Invariant
    /// - No two simulated agents can hold an active lease on the same path at the same time.
    pub async fn check_lock_exclusivity(fs: &SurrealFs) -> Result<()> {
        let mut res = fs
            .client()
            .query("SELECT file, path, holder, expires_at FROM file_lock WHERE expires_at > time::now();")
            .await?;

        let raw_locks: Vec<SValue> = res.take(0usize)?;
        let mut active_locks: Vec<LockNode> = Vec::new();
        for v in raw_locks {
            if let Ok(lock) = serde_json::from_value(v.into_json_value()) {
                active_locks.push(lock);
            }
        }

        let mut seen_files = HashSet::new();

        for lock in active_locks {
            let file_key = json_to_str(&lock.file);
            if seen_files.contains(&file_key) {
                bail!(
                    "Lock Exclusivity Invariant violated: multiple active locks on file {}",
                    file_key
                );
            }
            seen_files.insert(file_key);
        }

        Ok(())
    }

    /// Invariant 3: Branch Isolation Invariant
    /// - Files modified in branch `child` must not alter content in branch `base` before merge.
    pub async fn check_branch_isolation(
        fs: &SurrealFs,
        base_path: &str,
        expected_base_content: &str,
    ) -> Result<()> {
        let content = fs.read_text(base_path).await?;
        if content != expected_base_content {
            bail!(
                "Branch Isolation Invariant violated: base file {} content altered to '{}', expected '{}'",
                base_path,
                content,
                expected_base_content
            );
        }
        Ok(())
    }
}
