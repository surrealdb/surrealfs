//! Tree Export and Import supporting Tar, TarGz, and Zip formats (§23.3).

use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use crate::models::FileEntry;
use crate::paths::normalize_path;
use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use tar::{Archive, Builder, Header};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// Supported archive packaging formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    Tar,
    TarGz,
    Zip,
}

impl std::str::FromStr for ArchiveFormat {
    type Err = SurrealFsError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tar" => Ok(ArchiveFormat::Tar),
            "tar.gz" | "targz" | "tgz" => Ok(ArchiveFormat::TarGz),
            "zip" => Ok(ArchiveFormat::Zip),
            other => Err(SurrealFsError::InvalidArguments(format!(
                "Unsupported archive format: {}",
                other
            ))),
        }
    }
}

/// Statistics report for tree import operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportReport {
    pub files_imported: usize,
    pub dirs_created: usize,
    pub total_bytes: u64,
}

/// Helper to recursively collect all file entries under a root path.
pub async fn collect_tree(fs: &SurrealFs, root_path: &str) -> Result<Vec<FileEntry>> {
    let norm = normalize_path(root_path);
    let mut entries = Vec::new();
    let mut queue = vec![norm.clone()];

    while let Some(current_dir) = queue.pop() {
        let children = fs.ls(&current_dir).await?;
        for child in children {
            if child.is_folder {
                queue.push(child.path.clone());
            }
            entries.push(child);
        }
    }

    Ok(entries)
}

/// Export a SurrealFS directory tree to an archive in memory.
pub async fn export_tree(
    fs: &SurrealFs,
    root_path: &str,
    format: ArchiveFormat,
    _as_of: Option<DateTime<Utc>>,
) -> Result<Vec<u8>> {
    let norm_root = normalize_path(root_path);
    let entries = collect_tree(fs, &norm_root).await?;

    match format {
        ArchiveFormat::Tar => {
            let mut builder = Builder::new(Vec::new());
            append_entries_to_tar(fs, &mut builder, &norm_root, &entries).await?;
            let bytes = builder
                .into_inner()
                .map_err(|e| SurrealFsError::Storage(format!("Tar export failed: {}", e)))?;
            Ok(bytes)
        }
        ArchiveFormat::TarGz => {
            let mut gz = GzEncoder::new(Vec::new(), Compression::default());
            {
                let mut builder = Builder::new(&mut gz);
                append_entries_to_tar(fs, &mut builder, &norm_root, &entries).await?;
                builder
                    .finish()
                    .map_err(|e| SurrealFsError::Storage(format!("Tar finish failed: {}", e)))?;
            }
            let bytes = gz
                .finish()
                .map_err(|e| SurrealFsError::Storage(format!("Gzip finish failed: {}", e)))?;
            Ok(bytes)
        }
        ArchiveFormat::Zip => {
            let mut buf = Cursor::new(Vec::new());
            {
                let mut zip = ZipWriter::new(&mut buf);
                let options = SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);

                for entry in &entries {
                    let rel_path = entry
                        .path
                        .strip_prefix(&norm_root)
                        .unwrap_or(&entry.path)
                        .trim_start_matches('/');
                    if rel_path.is_empty() {
                        continue;
                    }

                    if entry.is_folder {
                        let dir_path = if rel_path.ends_with('/') {
                            rel_path.to_string()
                        } else {
                            format!("{}/", rel_path)
                        };
                        zip.add_directory(&dir_path, options).map_err(|e| {
                            SurrealFsError::Storage(format!("Zip directory failed: {}", e))
                        })?;
                    } else {
                        let data = fs.read_bytes(&entry.path).await?;
                        zip.start_file(rel_path, options).map_err(|e| {
                            SurrealFsError::Storage(format!("Zip start_file failed: {}", e))
                        })?;
                        zip.write_all(&data).map_err(|e| {
                            SurrealFsError::Storage(format!("Zip write failed: {}", e))
                        })?;
                    }
                }
                zip.finish()
                    .map_err(|e| SurrealFsError::Storage(format!("Zip finish failed: {}", e)))?;
            }
            Ok(buf.into_inner())
        }
    }
}

async fn append_entries_to_tar<W: Write>(
    fs: &SurrealFs,
    builder: &mut Builder<W>,
    root: &str,
    entries: &[FileEntry],
) -> Result<()> {
    for entry in entries {
        let rel_path = entry
            .path
            .strip_prefix(root)
            .unwrap_or(&entry.path)
            .trim_start_matches('/');
        if rel_path.is_empty() {
            continue;
        }

        let mut header = Header::new_gnu();
        let mtime = entry
            .updated_at
            .or(entry.created_at)
            .map(|t| t.timestamp() as u64)
            .unwrap_or(0);
        header.set_mtime(mtime);
        header.set_mode(entry.mode);

        if entry.is_folder {
            let dir_name = if rel_path.ends_with('/') {
                rel_path.to_string()
            } else {
                format!("{}/", rel_path)
            };
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder
                .append_data(&mut header, &dir_name, std::io::empty())
                .map_err(|e| SurrealFsError::Storage(format!("Tar append dir failed: {}", e)))?;
        } else {
            let data = fs.read_bytes(&entry.path).await?;
            header.set_size(data.len() as u64);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, rel_path, data.as_slice())
                .map_err(|e| SurrealFsError::Storage(format!("Tar append file failed: {}", e)))?;
        }
    }
    Ok(())
}

/// Import an archive into a SurrealFS destination folder.
pub async fn import_tree(
    fs: &SurrealFs,
    target_prefix: &str,
    archive_bytes: &[u8],
    format: ArchiveFormat,
) -> Result<ImportReport> {
    let base = normalize_path(target_prefix);
    let mut report = ImportReport::default();

    match format {
        ArchiveFormat::Tar => {
            let mut archive = Archive::new(Cursor::new(archive_bytes));
            unpack_tar(fs, &base, &mut archive, &mut report).await?;
        }
        ArchiveFormat::TarGz => {
            let gz = GzDecoder::new(Cursor::new(archive_bytes));
            let mut archive = Archive::new(gz);
            unpack_tar(fs, &base, &mut archive, &mut report).await?;
        }
        ArchiveFormat::Zip => {
            let reader = Cursor::new(archive_bytes);
            let mut zip = ZipArchive::new(reader)
                .map_err(|e| SurrealFsError::Storage(format!("Zip read failed: {}", e)))?;

            for i in 0..zip.len() {
                let mut file = zip.by_index(i).map_err(|e| {
                    SurrealFsError::Storage(format!("Zip entry extract failed: {}", e))
                })?;
                let raw_name = file
                    .name()
                    .map_err(|e| SurrealFsError::Storage(e.to_string()))?
                    .to_string();
                let clean_name = raw_name.trim_matches('/');
                if clean_name.is_empty() {
                    continue;
                }

                let dest_path = if base == "/" {
                    format!("/{}", clean_name)
                } else {
                    format!("{}/{}", base, clean_name)
                };

                if file.is_dir() {
                    fs.mkdir(&dest_path, true).await?;
                    report.dirs_created += 1;
                } else {
                    let mut content = Vec::new();
                    file.read_to_end(&mut content).map_err(|e| {
                        SurrealFsError::Storage(format!("Zip read content failed: {}", e))
                    })?;
                    report.total_bytes += content.len() as u64;
                    fs.write_bytes(&dest_path, &content, None).await?;
                    report.files_imported += 1;
                }
            }
        }
    }

    Ok(report)
}

async fn unpack_tar<R: Read>(
    fs: &SurrealFs,
    base: &str,
    archive: &mut Archive<R>,
    report: &mut ImportReport,
) -> Result<()> {
    let entries = archive
        .entries()
        .map_err(|e| SurrealFsError::Storage(format!("Tar entries failed: {}", e)))?;

    for entry_res in entries {
        let mut entry = entry_res
            .map_err(|e| SurrealFsError::Storage(format!("Tar read entry failed: {}", e)))?;
        let path_buf = entry
            .path()
            .map_err(|e| SurrealFsError::Storage(format!("Tar path parse failed: {}", e)))?;
        let raw_name = path_buf.to_string_lossy().to_string();
        let clean_name = raw_name.trim_matches('/');
        if clean_name.is_empty() {
            continue;
        }

        let dest_path = if base == "/" {
            format!("/{}", clean_name)
        } else {
            format!("{}/{}", base, clean_name)
        };

        if entry.header().entry_type().is_dir() {
            fs.mkdir(&dest_path, true).await?;
            report.dirs_created += 1;
        } else {
            let mut content = Vec::new();
            entry
                .read_to_end(&mut content)
                .map_err(|e| SurrealFsError::Storage(format!("Tar entry read failed: {}", e)))?;
            report.total_bytes += content.len() as u64;
            fs.write_bytes(&dest_path, &content, None).await?;
            report.files_imported += 1;
        }
    }
    Ok(())
}
