//! SFTP server subsystem implementation (§22.8).
//!
//! Enables legacy tools, scripts, and partners to access SurrealFS via SFTP
//! authenticating via SSH public keys against the `credential` table.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use surrealfs_core::fs::SurrealFs;
use surrealfs_core::models::FileEntry;
use surrealfs_core::paths::normalize_path;
use tokio::sync::RwLock;

// SFTP Protocol Packet Types (v3)
pub const SSH_FXP_INIT: u8 = 1;
pub const SSH_FXP_VERSION: u8 = 2;
pub const SSH_FXP_OPEN: u8 = 3;
pub const SSH_FXP_CLOSE: u8 = 4;
pub const SSH_FXP_READ: u8 = 5;
pub const SSH_FXP_WRITE: u8 = 6;
pub const SSH_FXP_LSTAT: u8 = 7;
pub const SSH_FXP_STAT: u8 = 17;
pub const SSH_FXP_OPENDIR: u8 = 11;
pub const SSH_FXP_READDIR: u8 = 12;
pub const SSH_FXP_REMOVE: u8 = 13;
pub const SSH_FXP_MKDIR: u8 = 14;
pub const SSH_FXP_RMDIR: u8 = 15;
pub const SSH_FXP_REALPATH: u8 = 16;
pub const SSH_FXP_RENAME: u8 = 18;

pub const SSH_FXP_STATUS: u8 = 101;
pub const SSH_FXP_HANDLE: u8 = 102;
pub const SSH_FXP_DATA: u8 = 103;
pub const SSH_FXP_NAME: u8 = 104;
pub const SSH_FXP_ATTRS: u8 = 105;

// SFTP Status Codes
pub const SSH_FX_OK: u32 = 0;
pub const SSH_FX_EOF: u32 = 1;
pub const SSH_FX_NO_SUCH_FILE: u32 = 2;
pub const SSH_FX_PERMISSION_DENIED: u32 = 3;
pub const SSH_FX_FAILURE: u32 = 4;
pub const SSH_FX_BAD_MESSAGE: u32 = 5;
pub const SSH_FX_NO_CONNECTION: u32 = 6;
pub const SSH_FX_CONNECTION_LOST: u32 = 7;
pub const SSH_FX_OP_UNSUPPORTED: u32 = 8;

// SFTP Open Flags (SSH_FXF_*)
pub const SSH_FXF_READ: u32 = 0x01;
pub const SSH_FXF_WRITE: u32 = 0x02;
pub const SSH_FXF_APPEND: u32 = 0x04;
pub const SSH_FXF_CREAT: u32 = 0x08;
pub const SSH_FXF_TRUNC: u32 = 0x10;
pub const SSH_FXF_EXCL: u32 = 0x20;

/// Open SFTP handle state (§22.8).
#[derive(Debug, Clone)]
struct SftpHandle {
    path: String,
    is_dir: bool,
    read_exhausted: bool,
    write_buffer: Vec<u8>,
    dirty: bool,
}

/// SFTP Subsystem session engine.
pub struct SftpSession {
    fs: SurrealFs,
    handles: Arc<RwLock<HashMap<String, SftpHandle>>>,
    next_handle_id: AtomicU64,
    caller: Arc<RwLock<Option<String>>>,
}

impl SftpSession {
    pub fn new(fs: SurrealFs) -> Self {
        Self {
            fs,
            handles: Arc::new(RwLock::new(HashMap::new())),
            next_handle_id: AtomicU64::new(1),
            caller: Arc::new(RwLock::new(None)),
        }
    }

    async fn scoped_fs(&self) -> SurrealFs {
        let caller = self.caller.read().await.clone();
        match caller {
            Some(c) => self.fs.with_caller(&c),
            None => self.fs.clone(),
        }
    }

    /// Authenticates an SSH public key against the `credential` store (§22.8).
    pub async fn authenticate_pubkey(&self, pubkey_fingerprint: &str) -> bool {
        match self.fs.resolve_credential("ssh", pubkey_fingerprint).await {
            Ok(Some(cred)) if cred.enabled => {
                *self.caller.write().await = Some(cred.user_id);
                true
            }
            _ => false,
        }
    }

    /// Dispatches an incoming binary SFTP packet and returns the response packet.
    pub async fn process_packet(&self, packet: &[u8]) -> Vec<u8> {
        if packet.is_empty() {
            return Vec::new();
        }

        let packet_type = packet[0];
        let payload = &packet[1..];

        match packet_type {
            SSH_FXP_INIT => {
                // Return SSH_FXP_VERSION with version 3
                let mut resp = vec![SSH_FXP_VERSION];
                resp.extend_from_slice(&3u32.to_be_bytes());
                wrap_packet(resp)
            }
            SSH_FXP_REALPATH => self.handle_realpath(payload).await,
            SSH_FXP_STAT | SSH_FXP_LSTAT => self.handle_stat(payload).await,
            SSH_FXP_OPENDIR => self.handle_opendir(payload).await,
            SSH_FXP_READDIR => self.handle_readdir(payload).await,
            SSH_FXP_OPEN => self.handle_open(payload).await,
            SSH_FXP_READ => self.handle_read(payload).await,
            SSH_FXP_WRITE => self.handle_write(payload).await,
            SSH_FXP_CLOSE => self.handle_close(payload).await,
            SSH_FXP_REMOVE => self.handle_remove(payload).await,
            SSH_FXP_MKDIR => self.handle_mkdir(payload).await,
            SSH_FXP_RMDIR => self.handle_rmdir(payload).await,
            SSH_FXP_RENAME => self.handle_rename(payload).await,
            _ => {
                let id = parse_u32(payload, 0).unwrap_or(0);
                build_status(id, SSH_FX_OP_UNSUPPORTED, "Unsupported SFTP operation")
            }
        }
    }

    async fn handle_realpath(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = parse_string(payload, 4).unwrap_or_else(|| "/".to_string());
        let normalized = normalize_path(&path);

        let mut resp = vec![SSH_FXP_NAME];
        resp.extend_from_slice(&id.to_be_bytes());
        resp.extend_from_slice(&1u32.to_be_bytes()); // Count = 1
        append_string(&mut resp, &normalized);
        append_string(&mut resp, &normalized); // Longname
        resp.extend_from_slice(&0u32.to_be_bytes()); // Empty dummy attrs
        wrap_packet(resp)
    }

    async fn handle_stat(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };

        let fs = self.scoped_fs().await;
        match fs.stat(&path).await {
            Ok(Some(stat)) => {
                let mut resp = vec![SSH_FXP_ATTRS];
                resp.extend_from_slice(&id.to_be_bytes());
                append_attrs(&mut resp, &stat);
                wrap_packet(resp)
            }
            Ok(None) => build_status(id, SSH_FX_NO_SUCH_FILE, "File not found"),
            Err(e) => build_status(id, SSH_FX_FAILURE, &e.to_string()),
        }
    }

    async fn handle_opendir(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };

        let hid = self.next_handle_id.fetch_add(1, Ordering::SeqCst);
        let handle_str = format!("dir-{}", hid);

        let mut handles = self.handles.write().await;
        handles.insert(
            handle_str.clone(),
            SftpHandle {
                path,
                is_dir: true,
                read_exhausted: false,
                write_buffer: Vec::new(),
                dirty: false,
            },
        );

        let mut resp = vec![SSH_FXP_HANDLE];
        resp.extend_from_slice(&id.to_be_bytes());
        append_string(&mut resp, &handle_str);
        wrap_packet(resp)
    }

    async fn handle_readdir(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let handle_str = match parse_string(payload, 4) {
            Some(h) => h,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid handle"),
        };

        let dir_path = {
            let mut handles = self.handles.write().await;
            let handle = match handles.get_mut(&handle_str) {
                Some(h) => h,
                None => return build_status(id, SSH_FX_FAILURE, "Invalid handle"),
            };

            if handle.read_exhausted {
                return build_status(id, SSH_FX_EOF, "End of directory");
            }

            handle.read_exhausted = true;
            handle.path.clone()
        };

        let fs = self.scoped_fs().await;
        let children = fs.ls(&dir_path).await.unwrap_or_default();

        let mut resp = vec![SSH_FXP_NAME];
        resp.extend_from_slice(&id.to_be_bytes());
        resp.extend_from_slice(&(children.len() as u32).to_be_bytes());

        for child in children {
            append_string(&mut resp, &child.filename);
            let longname = format!(
                "{} {} {}",
                if child.is_folder { "d" } else { "-" },
                child.size,
                child.filename
            );
            append_string(&mut resp, &longname);
            append_attrs(&mut resp, &child);
        }

        wrap_packet(resp)
    }

    async fn handle_open(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let raw_path = match parse_string(payload, 4) {
            Some(p) => p,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };
        let path = normalize_path(&raw_path);
        let pflags_offset = 4 + 4 + raw_path.len();
        let pflags = parse_u32(payload, pflags_offset).unwrap_or(SSH_FXF_CREAT | SSH_FXF_WRITE);

        let fs = self.scoped_fs().await;
        let existing = fs.stat(&path).await.ok().flatten();

        if (pflags & SSH_FXF_EXCL) != 0 && (pflags & SSH_FXF_CREAT) != 0 && existing.is_some() {
            return build_status(id, SSH_FX_FAILURE, "File already exists");
        }
        if (pflags & SSH_FXF_CREAT) == 0 && (pflags & SSH_FXF_WRITE) == 0 && existing.is_none() {
            return build_status(id, SSH_FX_NO_SUCH_FILE, "File not found");
        }

        let write_buffer = if (pflags & SSH_FXF_TRUNC) == 0 && existing.is_some() {
            fs.read_bytes(&path).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        let dirty =
            (pflags & SSH_FXF_TRUNC) != 0 || ((pflags & SSH_FXF_CREAT) != 0 && existing.is_none());

        let hid = self.next_handle_id.fetch_add(1, Ordering::SeqCst);
        let handle_str = format!("file-{}", hid);

        let mut handles = self.handles.write().await;
        handles.insert(
            handle_str.clone(),
            SftpHandle {
                path,
                is_dir: false,
                read_exhausted: false,
                write_buffer,
                dirty,
            },
        );

        let mut resp = vec![SSH_FXP_HANDLE];
        resp.extend_from_slice(&id.to_be_bytes());
        append_string(&mut resp, &handle_str);
        wrap_packet(resp)
    }

    async fn handle_read(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let handle_str = match parse_string(payload, 4) {
            Some(h) => h,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid handle"),
        };

        let handle_len = 4 + 4 + handle_str.len();
        let offset = parse_u64(payload, handle_len).unwrap_or(0);
        let len = parse_u32(payload, handle_len + 8).unwrap_or(32768);

        let handle_path = {
            let handles = self.handles.read().await;
            match handles.get(&handle_str) {
                Some(h) => h.path.clone(),
                None => return build_status(id, SSH_FX_FAILURE, "Invalid handle"),
            }
        };

        let fs = self.scoped_fs().await;
        match fs.read_range(&handle_path, offset, len as u64).await {
            Ok(data) => {
                if data.is_empty() {
                    build_status(id, SSH_FX_EOF, "EOF")
                } else {
                    let mut resp = vec![SSH_FXP_DATA];
                    resp.extend_from_slice(&id.to_be_bytes());
                    resp.extend_from_slice(&(data.len() as u32).to_be_bytes());
                    resp.extend_from_slice(&data);
                    wrap_packet(resp)
                }
            }
            Err(_) => build_status(id, SSH_FX_FAILURE, "Read error"),
        }
    }

    async fn handle_write(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let handle_str = match parse_string(payload, 4) {
            Some(h) => h,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid handle"),
        };

        let handle_len = 4 + 4 + handle_str.len();
        let offset = parse_u64(payload, handle_len).unwrap_or(0);
        let data_offset = handle_len + 8;
        let data = match parse_byte_slice(payload, data_offset) {
            Some(d) => d,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid data chunk"),
        };

        let mut handles = self.handles.write().await;
        let handle = match handles.get_mut(&handle_str) {
            Some(h) => h,
            None => return build_status(id, SSH_FX_FAILURE, "Invalid handle"),
        };

        let end = (offset as usize) + data.len();
        if handle.write_buffer.len() < end {
            handle.write_buffer.resize(end, 0);
        }
        handle.write_buffer[offset as usize..end].copy_from_slice(data);
        handle.dirty = true;

        build_status(id, SSH_FX_OK, "OK")
    }

    async fn handle_close(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let handle_str = match parse_string(payload, 4) {
            Some(h) => h,
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid handle"),
        };

        let handle = {
            let mut handles = self.handles.write().await;
            handles.remove(&handle_str)
        };

        if let Some(handle) = handle {
            if !handle.is_dir && (handle.dirty || !handle.write_buffer.is_empty()) {
                let fs = self.scoped_fs().await;
                if let Err(e) = fs
                    .write_bytes(&handle.path, &handle.write_buffer, None)
                    .await
                {
                    return build_status(id, SSH_FX_FAILURE, &e.to_string());
                }
            }
        }

        build_status(id, SSH_FX_OK, "OK")
    }

    async fn handle_remove(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };

        let fs = self.scoped_fs().await;
        match fs.rm(&path, false).await {
            Ok(_) => build_status(id, SSH_FX_OK, "OK"),
            Err(e) => build_status(id, SSH_FX_FAILURE, &e.to_string()),
        }
    }

    async fn handle_mkdir(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };

        let fs = self.scoped_fs().await;
        match fs.mkdir(&path, false).await {
            Ok(_) => build_status(id, SSH_FX_OK, "OK"),
            Err(e) => build_status(id, SSH_FX_FAILURE, &e.to_string()),
        }
    }

    async fn handle_rmdir(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let path = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid path"),
        };

        let fs = self.scoped_fs().await;
        match fs.rm(&path, true).await {
            Ok(_) => build_status(id, SSH_FX_OK, "OK"),
            Err(e) => build_status(id, SSH_FX_FAILURE, &e.to_string()),
        }
    }

    async fn handle_rename(&self, payload: &[u8]) -> Vec<u8> {
        let id = parse_u32(payload, 0).unwrap_or(0);
        let oldpath = match parse_string(payload, 4) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid old path"),
        };
        let old_len = 4 + 4 + oldpath.len();
        let newpath = match parse_string(payload, old_len) {
            Some(p) => normalize_path(&p),
            None => return build_status(id, SSH_FX_BAD_MESSAGE, "Invalid new path"),
        };

        let fs = self.scoped_fs().await;
        match fs.mv(&oldpath, &newpath).await {
            Ok(_) => build_status(id, SSH_FX_OK, "OK"),
            Err(e) => build_status(id, SSH_FX_FAILURE, &e.to_string()),
        }
    }
}

fn wrap_packet(payload: Vec<u8>) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(4 + payload.len());
    pkt.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    pkt.extend_from_slice(&payload);
    pkt
}

fn build_status(id: u32, status_code: u32, message: &str) -> Vec<u8> {
    let mut resp = vec![SSH_FXP_STATUS];
    resp.extend_from_slice(&id.to_be_bytes());
    resp.extend_from_slice(&status_code.to_be_bytes());
    append_string(&mut resp, message);
    append_string(&mut resp, "en"); // Language tag
    wrap_packet(resp)
}

fn append_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn append_attrs(buf: &mut Vec<u8>, entry: &FileEntry) {
    // Flags: SSH_FILEXFER_ATTR_SIZE (0x01) | SSH_FILEXFER_ATTR_PERMISSIONS (0x04)
    let flags = 0x01u32 | 0x04u32;
    buf.extend_from_slice(&flags.to_be_bytes());
    buf.extend_from_slice(&entry.size.to_be_bytes());
    let mode = if entry.is_folder {
        entry.mode | 0o040000
    } else {
        entry.mode | 0o100000
    };
    buf.extend_from_slice(&mode.to_be_bytes());
}

fn parse_u32(buf: &[u8], offset: usize) -> Option<u32> {
    if buf.len() < offset + 4 {
        return None;
    }
    let bytes: [u8; 4] = buf[offset..offset + 4].try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

fn parse_u64(buf: &[u8], offset: usize) -> Option<u64> {
    if buf.len() < offset + 8 {
        return None;
    }
    let bytes: [u8; 8] = buf[offset..offset + 8].try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

fn parse_string(buf: &[u8], offset: usize) -> Option<String> {
    let len = parse_u32(buf, offset)? as usize;
    let start = offset + 4;
    let end = start + len;
    if buf.len() < end {
        return None;
    }
    String::from_utf8(buf[start..end].to_vec()).ok()
}

fn parse_byte_slice(buf: &[u8], offset: usize) -> Option<&[u8]> {
    let len = parse_u32(buf, offset)? as usize;
    let start = offset + 4;
    let end = start + len;
    if buf.len() < end {
        return None;
    }
    Some(&buf[start..end])
}
