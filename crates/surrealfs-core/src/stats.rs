//! System Observability, Metrics and Virtual Filesystem Stats (§24.6).

use crate::errors::Result;
use crate::fs::SurrealFs;
use serde::{Deserialize, Serialize};

/// Comprehensive statistics across SurrealFS filesystem tables.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FsStats {
    pub total_files: usize,
    pub total_directories: usize,
    pub total_bytes: u64,
    pub total_chunks: usize,
    pub dedup_ratio: f64,
    pub active_shares: usize,
    pub active_webhooks: usize,
    pub groups_count: usize,
    pub legal_holds_count: usize,
}

impl SurrealFs {
    /// Compute and aggregate system-wide statistics for SurrealFS.
    pub async fn get_stats(&self) -> Result<FsStats> {
        let sql = r#"
            LET $files = (SELECT is_folder,
                                 IF size IS NOT NONE THEN size
                                 ELSE IF content IS NOT NONE THEN string::len(content)
                                 ELSE IF file IS NOT NONE THEN bytes::len(file)
                                 ELSE 0 END AS size,
                                 meta.legal_hold AS hold FROM file WHERE deleted_at IS NONE);
            LET $fc = (SELECT count() AS c, math::sum(length ?? byte_length ?? 0) AS s FROM file_chunk GROUP ALL)[0];
            LET $blobs = (SELECT count() AS c, math::sum(stored ?? size ?? 0) AS p FROM blob GROUP ALL)[0];
            LET $shares = (SELECT count() FROM share WHERE revoked = false GROUP ALL)[0].count ?? 0;
            LET $hooks = (SELECT count() FROM webhook WHERE active = true GROUP ALL)[0].count ?? 0;
            LET $groups = (SELECT count() FROM user_group GROUP ALL)[0].count ?? 0;
            RETURN {
                files: $files,
                chunks: ($fc.c ?? 0) + ($blobs.c ?? 0),
                logical_chunk_bytes: $fc.s ?? 0,
                physical_blob_bytes: $blobs.p ?? 0,
                shares: $shares,
                hooks: $hooks,
                groups: $groups
            };
        "#;

        let mut res = self.db.query(sql).await?;
        let data: Option<serde_json::Value> = res.take(6usize)?;

        let mut stats = FsStats::default();

        if let Some(obj) = data {
            if let Some(files_arr) = obj.get("files").and_then(|v| v.as_array()) {
                for item in files_arr {
                    let is_dir = item
                        .get("is_folder")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                    let hold = item.get("hold").and_then(|v| v.as_bool()).unwrap_or(false);

                    if is_dir {
                        stats.total_directories += 1;
                    } else {
                        stats.total_files += 1;
                        stats.total_bytes += size;
                    }

                    if hold {
                        stats.legal_holds_count += 1;
                    }
                }
            }

            stats.total_chunks = obj.get("chunks").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            stats.active_shares = obj.get("shares").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            stats.active_webhooks = obj.get("hooks").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            stats.groups_count = obj.get("groups").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

            let logical_chunk_bytes = obj
                .get("logical_chunk_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let physical_blob_bytes = obj
                .get("physical_blob_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if physical_blob_bytes > 0 {
                let logical = if logical_chunk_bytes > 0 {
                    logical_chunk_bytes
                } else {
                    stats.total_bytes
                };
                stats.dedup_ratio = (logical as f64) / (physical_blob_bytes as f64);
            } else {
                stats.dedup_ratio = 1.0;
            }
        }

        Ok(stats)
    }

    /// Check if a path corresponds to a virtual observability file (e.g. `/.surrealfs/stats`).
    pub async fn read_virtual_stats(&self, path: &str) -> Option<String> {
        let clean = path.trim_start_matches('/');
        if clean == ".surrealfs/stats" || clean == "stats" {
            if let Ok(stats) = self.get_stats().await {
                return serde_json::to_string_pretty(&stats).ok();
            }
        } else if clean == ".surrealfs/metrics" || clean == "metrics" {
            if let Ok(stats) = self.get_stats().await {
                let prometheus = format!(
                    "# HELP surrealfs_files_total Total regular files\n\
                     # TYPE surrealfs_files_total gauge\n\
                     surrealfs_files_total {}\n\
                     # HELP surrealfs_directories_total Total directories\n\
                     # TYPE surrealfs_directories_total gauge\n\
                     surrealfs_directories_total {}\n\
                     # HELP surrealfs_bytes_total Total size in bytes\n\
                     # TYPE surrealfs_bytes_total gauge\n\
                     surrealfs_bytes_total {}\n\
                     # HELP surrealfs_chunks_total Total deduplicated chunks\n\
                     # TYPE surrealfs_chunks_total gauge\n\
                     surrealfs_chunks_total {}\n\
                     # HELP surrealfs_active_shares Active share links\n\
                     # TYPE surrealfs_active_shares gauge\n\
                     surrealfs_active_shares {}\n\
                     # HELP surrealfs_active_webhooks Active registered webhooks\n\
                     # TYPE surrealfs_active_webhooks gauge\n\
                     surrealfs_active_webhooks {}\n\
                     # HELP surrealfs_groups_total Total user groups\n\
                     # TYPE surrealfs_groups_total gauge\n\
                     surrealfs_groups_total {}\n\
                     # HELP surrealfs_legal_holds Active legal holds\n\
                     # TYPE surrealfs_legal_holds gauge\n\
                     surrealfs_legal_holds {}\n",
                    stats.total_files,
                    stats.total_directories,
                    stats.total_bytes,
                    stats.total_chunks,
                    stats.active_shares,
                    stats.active_webhooks,
                    stats.groups_count,
                    stats.legal_holds_count
                );
                return Some(prometheus);
            }
        }
        None
    }
}
