//! Content-Addressed Backup and Point-in-Time Restore Engine (§24.5).

use crate::archive::collect_tree;
use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use crate::paths::normalize_path;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::{Read, Write};
use std::path::Path;

/// Manifest header describing an incremental SurrealFS backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub root_path: String,
    pub files: Vec<BackupFileEntry>,
    pub total_bytes: u64,
}

/// Metadata and blob reference for a single file in the backup manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFileEntry {
    pub path: String,
    pub is_folder: bool,
    pub mode: u32,
    pub owner: String,
    pub group: Option<String>,
    pub size: u64,
    pub hash: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub meta: Option<serde_json::Value>,
}

/// Report summarizing restore execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreReport {
    pub files_restored: usize,
    pub dirs_restored: usize,
    pub bytes_restored: u64,
}

/// Create a content-addressed backup to the specified destination directory.
pub async fn create_backup(
    fs: &SurrealFs,
    root_path: &str,
    backup_dir: &Path,
) -> Result<BackupManifest> {
    let norm_root = normalize_path(root_path);
    let entries = collect_tree(fs, &norm_root).await?;

    let blobs_dir = backup_dir.join("blobs");
    create_dir_all(&blobs_dir)
        .map_err(|e| SurrealFsError::Io(format!("Failed to create blobs directory: {}", e)))?;

    let mut manifest_files = Vec::new();
    let mut total_bytes = 0u64;

    for entry in entries {
        let mut file_hash = None;

        if !entry.is_folder {
            let data = fs.read_bytes(&entry.path).await?;
            let hash = blake3::hash(&data).to_hex().to_string();
            let blob_path = blobs_dir.join(format!("{}.blob", hash));

            // Only write blob if not already backed up (content-addressed deduplication)
            if !blob_path.exists() {
                let mut f = File::create(&blob_path).map_err(|e| {
                    SurrealFsError::Io(format!(
                        "Failed to write blob {}: {}",
                        blob_path.display(),
                        e
                    ))
                })?;
                f.write_all(&data)
                    .map_err(|e| SurrealFsError::Io(format!("Failed to write blob data: {}", e)))?;
            }

            total_bytes += data.len() as u64;
            file_hash = Some(hash);
        }

        manifest_files.push(BackupFileEntry {
            path: entry.path,
            is_folder: entry.is_folder,
            mode: entry.mode,
            owner: entry.owner.unwrap_or_else(|| "root".to_string()),
            group: entry.group,
            size: entry.size,
            hash: file_hash,
            created_at: entry.created_at,
            updated_at: entry.updated_at,
            meta: entry.meta,
        });
    }

    let manifest = BackupManifest {
        version: 1,
        created_at: Utc::now(),
        root_path: norm_root,
        files: manifest_files,
        total_bytes,
    };

    let manifest_path = backup_dir.join("manifest.json");
    let json_bytes = serde_json::to_vec_pretty(&manifest)?;
    let mut f = File::create(&manifest_path).map_err(|e| {
        SurrealFsError::Io(format!(
            "Failed to write manifest {}: {}",
            manifest_path.display(),
            e
        ))
    })?;
    f.write_all(&json_bytes)
        .map_err(|e| SurrealFsError::Io(format!("Failed to flush manifest: {}", e)))?;

    Ok(manifest)
}

/// Restore a backup to a target prefix, optionally filtering as of a specific point in time.
pub async fn restore_backup(
    fs: &SurrealFs,
    backup_dir: &Path,
    target_prefix: &str,
    as_of: Option<DateTime<Utc>>,
) -> Result<RestoreReport> {
    let manifest_path = backup_dir.join("manifest.json");
    let mut f = File::open(&manifest_path)
        .map_err(|e| SurrealFsError::NotFound(format!("Backup manifest not found: {}", e)))?;
    let mut manifest_data = Vec::new();
    f.read_to_end(&mut manifest_data)
        .map_err(|e| SurrealFsError::Io(format!("Failed to read backup manifest: {}", e)))?;

    let manifest: BackupManifest = serde_json::from_slice(&manifest_data)?;
    let blobs_dir = backup_dir.join("blobs");
    let base = normalize_path(target_prefix);

    let mut report = RestoreReport::default();

    for file in manifest.files {
        // Point-in-time filter: if created after as_of, skip
        if let Some(cutoff) = as_of {
            if let Some(created) = file.created_at {
                if created > cutoff {
                    continue;
                }
            }
        }

        let rel_path = file
            .path
            .strip_prefix(&manifest.root_path)
            .unwrap_or(&file.path)
            .trim_start_matches('/');

        let dest_path = if base == "/" {
            format!("/{}", rel_path)
        } else {
            format!("{}/{}", base, rel_path)
        };

        if file.is_folder {
            fs.mkdir(&dest_path, true).await?;
            if let Some(ref grp) = file.group {
                let _ = fs.chgrp(&dest_path, Some(grp)).await;
            }
            let _ = fs.chmod(&dest_path, file.mode).await;
            report.dirs_restored += 1;
        } else if let Some(ref hash) = file.hash {
            let blob_path = blobs_dir.join(format!("{}.blob", hash));
            let mut bf = File::open(&blob_path)
                .map_err(|e| SurrealFsError::NotFound(format!("Blob {} missing: {}", hash, e)))?;
            let mut blob_bytes = Vec::new();
            bf.read_to_end(&mut blob_bytes)
                .map_err(|e| SurrealFsError::Io(format!("Failed to read blob {}: {}", hash, e)))?;

            fs.write_bytes(&dest_path, &blob_bytes, None).await?;
            if let Some(ref grp) = file.group {
                let _ = fs.chgrp(&dest_path, Some(grp)).await;
            }
            let _ = fs.chmod(&dest_path, file.mode).await;

            report.files_restored += 1;
            report.bytes_restored += blob_bytes.len() as u64;
        }
    }

    Ok(report)
}
