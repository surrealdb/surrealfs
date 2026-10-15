//! NFSv3 loopback server implementation (§22.2).
//!
//! Enables native Unix mount on macOS and Linux without FUSE or kernel
//! extensions, mounting through the OS built-in NFS client over localhost.

use chrono::Utc;
use surrealfs_core::errors::SurrealFsError;
use surrealfs_core::fs::SurrealFs;
use surrealfs_core::models::FileEntry;
use surrealfs_core::paths::normalize_path;

/// NFSv3 Status Codes (RFC 1813).
pub const NFS3_OK: u32 = 0;
pub const NFS3ERR_PERM: u32 = 1;
pub const NFS3ERR_NOENT: u32 = 2;
pub const NFS3ERR_IO: u32 = 5;
pub const NFS3ERR_NXIO: u32 = 6;
pub const NFS3ERR_ACCES: u32 = 13;
pub const NFS3ERR_EXIST: u32 = 17;
pub const NFS3ERR_NOTDIR: u32 = 20;
pub const NFS3ERR_ISDIR: u32 = 21;
pub const NFS3ERR_INVAL: u32 = 22;
pub const NFS3ERR_FBIG: u32 = 27;
pub const NFS3ERR_NOSPC: u32 = 28;
pub const NFS3ERR_ROFS: u32 = 30;
pub const NFS3ERR_NAMETOOLONG: u32 = 63;
pub const NFS3ERR_NOTEMPTY: u32 = 66;
pub const NFS3ERR_STALE: u32 = 70; // ESTALE: Generation or record mismatch (§22.2)
pub const NFS3ERR_BADHANDLE: u32 = 10001;

/// NFSv3 File Types.
pub const NF3REG: u32 = 1;
pub const NF3DIR: u32 = 2;

/// Stateless NFSv3 File Handle (§22.2).
/// Encodes the target filesystem path and generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfsFileHandle {
    pub path: String,
    pub generation: u64,
}

impl NfsFileHandle {
    pub fn new(path: &str, generation: u64) -> Self {
        Self {
            path: normalize_path(path),
            generation,
        }
    }

    /// Serializes handle to raw wire bytes (max 64 bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.generation.to_be_bytes());
        buf.extend_from_slice(self.path.as_bytes());
        buf
    }

    /// Parses handle from raw wire bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            return None;
        }
        let gen_bytes: [u8; 8] = bytes[..8].try_into().ok()?;
        let generation = u64::from_be_bytes(gen_bytes);
        let path = String::from_utf8(bytes[8..].to_vec()).ok()?;
        Some(Self { path, generation })
    }
}

/// NFSv3 File Attributes.
#[derive(Debug, Clone)]
pub struct NfsFattr3 {
    pub file_type: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub used: u64,
    pub rdev: (u32, u32),
    pub fsid: u64,
    pub fileid: u64,
    pub atime: (u32, u32),
    pub mtime: (u32, u32),
    pub ctime: (u32, u32),
}

impl NfsFattr3 {
    pub fn from_entry(entry: &FileEntry) -> Self {
        let file_type = if entry.is_folder { NF3DIR } else { NF3REG };
        let mode = if entry.is_folder {
            entry.mode | 0o040000 // S_IFDIR
        } else {
            entry.mode | 0o100000 // S_IFREG
        };

        let now_sec = Utc::now().timestamp() as u32;
        let created_sec = entry
            .created_at
            .map(|t| t.timestamp() as u32)
            .unwrap_or(now_sec);
        let updated_sec = entry
            .updated_at
            .map(|t| t.timestamp() as u32)
            .unwrap_or(now_sec);

        let fileid = blake3::hash(entry.path.as_bytes()).as_bytes()[..8]
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | (b as u64));

        Self {
            file_type,
            mode,
            nlink: if entry.is_folder { 2 } else { 1 },
            uid: 501,
            gid: 20,
            size: entry.size,
            used: entry.size,
            rdev: (0, 0),
            fsid: 42,
            fileid,
            atime: (now_sec, 0),
            mtime: (updated_sec, 0),
            ctime: (created_sec, 0),
        }
    }
}

/// NFSv3 Server engine (§22.2).
pub struct NfsServer {
    fs: SurrealFs,
}

impl NfsServer {
    pub fn new(fs: SurrealFs) -> Self {
        Self { fs }
    }

    /// Verifies that a file handle is still valid and not stale (§22.2).
    /// Returns ESTALE if the file was replaced concurrently.
    pub async fn verify_handle(&self, handle: &NfsFileHandle) -> Result<FileEntry, u32> {
        let stat = match self.fs.stat(&handle.path).await {
            Ok(Some(s)) => s,
            Ok(None) if handle.path == "/" => FileEntry {
                path: "/".to_string(),
                filename: "".to_string(),
                is_folder: true,
                size: 0,
                content_type: "inode/directory".to_string(),
                content: None,
                mode: 0o755,
                owner: Some("root".to_string()),
                group: None,
                branch: "main".to_string(),
                generation: 1,
                parent_key: None,
                crdt: false,
                created_at: Some(Utc::now()),
                updated_at: Some(Utc::now()),
                meta: None,
            },
            Ok(None) => return Err(NFS3ERR_NOENT),
            Err(_) => return Err(NFS3ERR_IO),
        };

        // Check optimistic generation match: handle to a replaced file returns ESTALE
        if handle.generation != 0 && stat.generation != handle.generation {
            return Err(NFS3ERR_STALE);
        }

        Ok(stat)
    }

    /// NFS3 Procedure: GETATTR.
    pub async fn getattr(&self, handle: &NfsFileHandle) -> Result<NfsFattr3, u32> {
        let entry = self.verify_handle(handle).await?;
        Ok(NfsFattr3::from_entry(&entry))
    }

    /// NFS3 Procedure: LOOKUP.
    pub async fn lookup(
        &self,
        dir_handle: &NfsFileHandle,
        name: &str,
    ) -> Result<(NfsFileHandle, NfsFattr3), u32> {
        let _ = self.verify_handle(dir_handle).await?;
        let child_path = if dir_handle.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", dir_handle.path, name)
        };

        let stat = match self.fs.stat(&child_path).await {
            Ok(Some(s)) => s,
            Ok(None) => return Err(NFS3ERR_NOENT),
            Err(_) => return Err(NFS3ERR_IO),
        };

        let handle = NfsFileHandle::new(&stat.path, stat.generation);
        let fattr = NfsFattr3::from_entry(&stat);
        Ok((handle, fattr))
    }

    /// NFS3 Procedure: READ.
    pub async fn read(
        &self,
        handle: &NfsFileHandle,
        offset: u64,
        count: u32,
    ) -> Result<(Vec<u8>, bool), u32> {
        let stat = self.verify_handle(handle).await?;
        if stat.is_folder {
            return Err(NFS3ERR_ISDIR);
        }

        let data = match self.fs.read_range(&handle.path, offset, count as u64).await {
            Ok(d) => d,
            Err(_) => {
                let all = self
                    .fs
                    .read_bytes(&handle.path)
                    .await
                    .map_err(|_| NFS3ERR_IO)?;
                let start = (offset as usize).min(all.len());
                let end = ((offset + count as u64) as usize).min(all.len());
                all[start..end].to_vec()
            }
        };

        let eof = (offset + data.len() as u64) >= stat.size;
        Ok((data, eof))
    }

    /// NFS3 Procedure: WRITE.
    pub async fn write(
        &self,
        handle: &NfsFileHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(u32, NfsFattr3), u32> {
        let stat = self.verify_handle(handle).await?;
        if stat.is_folder {
            return Err(NFS3ERR_ISDIR);
        }

        let mut current = self.fs.read_bytes(&handle.path).await.unwrap_or_default();
        let end_offset = (offset as usize) + data.len();
        if current.len() < end_offset {
            current.resize(end_offset, 0);
        }
        current[offset as usize..end_offset].copy_from_slice(data);

        let updated = self
            .fs
            .write_bytes(&handle.path, &current, Some(handle.generation))
            .await
            .map_err(|e| match e {
                SurrealFsError::Conflict(_) => NFS3ERR_STALE,
                _ => NFS3ERR_IO,
            })?;

        Ok((data.len() as u32, NfsFattr3::from_entry(&updated)))
    }

    /// NFS3 Procedure: CREATE.
    pub async fn create(
        &self,
        dir_handle: &NfsFileHandle,
        name: &str,
    ) -> Result<(NfsFileHandle, NfsFattr3), u32> {
        let _ = self.verify_handle(dir_handle).await?;
        let child_path = if dir_handle.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", dir_handle.path, name)
        };

        let created = self
            .fs
            .write_bytes(&child_path, &[], None)
            .await
            .map_err(|_| NFS3ERR_IO)?;

        let handle = NfsFileHandle::new(&created.path, created.generation);
        let fattr = NfsFattr3::from_entry(&created);
        Ok((handle, fattr))
    }

    /// NFS3 Procedure: MKDIR.
    pub async fn mkdir(
        &self,
        dir_handle: &NfsFileHandle,
        name: &str,
    ) -> Result<(NfsFileHandle, NfsFattr3), u32> {
        let _ = self.verify_handle(dir_handle).await?;
        let child_path = if dir_handle.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", dir_handle.path, name)
        };

        self.fs
            .mkdir(&child_path, false)
            .await
            .map_err(|e| match e {
                SurrealFsError::Conflict(_) => NFS3ERR_EXIST,
                _ => NFS3ERR_IO,
            })?;

        let stat = match self.fs.stat(&child_path).await {
            Ok(Some(s)) => s,
            _ => return Err(NFS3ERR_IO),
        };
        let handle = NfsFileHandle::new(&stat.path, stat.generation);
        let fattr = NfsFattr3::from_entry(&stat);
        Ok((handle, fattr))
    }

    /// NFS3 Procedure: REMOVE.
    pub async fn remove(&self, dir_handle: &NfsFileHandle, name: &str) -> Result<(), u32> {
        let _ = self.verify_handle(dir_handle).await?;
        let child_path = if dir_handle.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", dir_handle.path, name)
        };

        self.fs
            .rm(&child_path, false)
            .await
            .map_err(|_| NFS3ERR_IO)?;
        Ok(())
    }

    /// NFS3 Procedure: READDIRPLUS.
    pub async fn readdirplus(
        &self,
        dir_handle: &NfsFileHandle,
    ) -> Result<Vec<(String, NfsFileHandle, NfsFattr3)>, u32> {
        let _ = self.verify_handle(dir_handle).await?;
        let children = self.fs.ls(&dir_handle.path).await.map_err(|_| NFS3ERR_IO)?;

        let mut res = Vec::new();
        for child in children {
            let handle = NfsFileHandle::new(&child.path, child.generation);
            let fattr = NfsFattr3::from_entry(&child);
            res.push((child.filename, handle, fattr));
        }
        Ok(res)
    }

    /// Generates OS loopback mount command for macOS and Linux (§22.2).
    pub fn mount_command(&self, mount_point: &str, port: u16) -> String {
        #[cfg(target_os = "macos")]
        {
            format!(
                "mount -t nfs -o port={},mountport={},tcp,noacl,locallocks,resvport 127.0.0.1:/ {}",
                port, port, mount_point
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            format!(
                "mount -t nfs -o port={},mountport={},tcp,nolock 127.0.0.1:/ {}",
                port, port, mount_point
            )
        }
    }

    /// Handles a single ONC RPC call message (without record-marking prefix) and returns the XDR reply.
    pub async fn handle_rpc_call(&self, call: &[u8]) -> Option<Vec<u8>> {
        if call.len() < 24 {
            return None;
        }
        let xid = u32::from_be_bytes(call[0..4].try_into().ok()?);
        let msg_type = u32::from_be_bytes(call[4..8].try_into().ok()?);
        if msg_type != 0 {
            return None;
        }
        let prog = u32::from_be_bytes(call[12..16].try_into().ok()?);
        let proc_num = u32::from_be_bytes(call[20..24].try_into().ok()?);

        // Skip auth cred and verf opaque fields
        let mut pos = 24usize;
        for _ in 0..2 {
            if pos + 8 > call.len() {
                return None;
            }
            let opaque_len = u32::from_be_bytes(call[pos + 4..pos + 8].try_into().ok()?) as usize;
            let padded = (opaque_len + 3) & !3;
            pos += 8 + padded;
        }
        if pos > call.len() {
            return None;
        }
        let args = &call[pos..];

        let mut reply = Vec::with_capacity(128);
        reply.extend_from_slice(&xid.to_be_bytes());
        reply.extend_from_slice(&1u32.to_be_bytes()); // REPLY
        reply.extend_from_slice(&0u32.to_be_bytes()); // MSG_ACCEPTED
        reply.extend_from_slice(&0u32.to_be_bytes()); // verf flavor AUTH_NULL
        reply.extend_from_slice(&0u32.to_be_bytes()); // verf len 0

        match (prog, proc_num) {
            (_, 0) => {
                // NULL procedure (NFS or MOUNT)
                reply.extend_from_slice(&0u32.to_be_bytes()); // SUCCESS
            }
            (100005, 1) => {
                // MOUNTPROC3_MNT: return root file handle
                reply.extend_from_slice(&0u32.to_be_bytes()); // SUCCESS
                reply.extend_from_slice(&0u32.to_be_bytes()); // MNT3_OK
                let root_fh = NfsFileHandle::new("/", 0).to_bytes();
                encode_xdr_opaque(&mut reply, &root_fh);
                reply.extend_from_slice(&1u32.to_be_bytes()); // 1 auth flavor
                reply.extend_from_slice(&1u32.to_be_bytes()); // AUTH_SYS
            }
            (100003, 1) => {
                // NFSPROC3_GETATTR
                reply.extend_from_slice(&0u32.to_be_bytes()); // SUCCESS
                let Some((fh_bytes, _)) = decode_xdr_opaque(args) else {
                    reply.extend_from_slice(&NFS3ERR_BADHANDLE.to_be_bytes());
                    return Some(reply);
                };
                let Some(fh) = NfsFileHandle::from_bytes(fh_bytes) else {
                    reply.extend_from_slice(&NFS3ERR_BADHANDLE.to_be_bytes());
                    return Some(reply);
                };
                match self.getattr(&fh).await {
                    Ok(fattr) => {
                        reply.extend_from_slice(&NFS3_OK.to_be_bytes());
                        encode_fattr3(&mut reply, &fattr);
                    }
                    Err(status) => {
                        reply.extend_from_slice(&status.to_be_bytes());
                    }
                }
            }
            (100003, 3) => {
                // NFSPROC3_LOOKUP
                reply.extend_from_slice(&0u32.to_be_bytes()); // SUCCESS
                let Some((fh_bytes, rest)) = decode_xdr_opaque(args) else {
                    reply.extend_from_slice(&NFS3ERR_BADHANDLE.to_be_bytes());
                    reply.extend_from_slice(&0u32.to_be_bytes()); // post_op_attr = FALSE
                    return Some(reply);
                };
                let Some((name_bytes, _)) = decode_xdr_opaque(rest) else {
                    reply.extend_from_slice(&NFS3ERR_INVAL.to_be_bytes());
                    reply.extend_from_slice(&0u32.to_be_bytes());
                    return Some(reply);
                };
                let (Some(dir_fh), Ok(name)) =
                    (NfsFileHandle::from_bytes(fh_bytes), std::str::from_utf8(name_bytes))
                else {
                    reply.extend_from_slice(&NFS3ERR_BADHANDLE.to_be_bytes());
                    reply.extend_from_slice(&0u32.to_be_bytes());
                    return Some(reply);
                };
                match self.lookup(&dir_fh, name).await {
                    Ok((child_fh, fattr)) => {
                        reply.extend_from_slice(&NFS3_OK.to_be_bytes());
                        encode_xdr_opaque(&mut reply, &child_fh.to_bytes());
                        reply.extend_from_slice(&1u32.to_be_bytes()); // obj_attributes present
                        encode_fattr3(&mut reply, &fattr);
                        reply.extend_from_slice(&0u32.to_be_bytes()); // dir_attributes omitted
                    }
                    Err(status) => {
                        reply.extend_from_slice(&status.to_be_bytes());
                        reply.extend_from_slice(&0u32.to_be_bytes());
                    }
                }
            }
            _ => {
                // PROC_UNAVAIL = 3
                reply.extend_from_slice(&3u32.to_be_bytes());
            }
        }
        Some(reply)
    }
}

fn decode_xdr_opaque(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes(buf[0..4].try_into().ok()?) as usize;
    let padded = (len + 3) & !3;
    if buf.len() < 4 + padded {
        return None;
    }
    Some((&buf[4..4 + len], &buf[4 + padded..]))
}

fn encode_xdr_opaque(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
    let pad = (4 - (bytes.len() % 4)) % 4;
    for _ in 0..pad {
        out.push(0);
    }
}

fn encode_fattr3(out: &mut Vec<u8>, f: &NfsFattr3) {
    out.extend_from_slice(&f.file_type.to_be_bytes());
    out.extend_from_slice(&f.mode.to_be_bytes());
    out.extend_from_slice(&f.nlink.to_be_bytes());
    out.extend_from_slice(&f.uid.to_be_bytes());
    out.extend_from_slice(&f.gid.to_be_bytes());
    out.extend_from_slice(&f.size.to_be_bytes());
    out.extend_from_slice(&f.used.to_be_bytes());
    out.extend_from_slice(&f.rdev.0.to_be_bytes());
    out.extend_from_slice(&f.rdev.1.to_be_bytes());
    out.extend_from_slice(&f.fsid.to_be_bytes());
    out.extend_from_slice(&f.fileid.to_be_bytes());
    out.extend_from_slice(&f.atime.0.to_be_bytes());
    out.extend_from_slice(&f.atime.1.to_be_bytes());
    out.extend_from_slice(&f.mtime.0.to_be_bytes());
    out.extend_from_slice(&f.mtime.1.to_be_bytes());
    out.extend_from_slice(&f.ctime.0.to_be_bytes());
    out.extend_from_slice(&f.ctime.1.to_be_bytes());
}

/// Runs the NFSv3 over TCP loopback listener on `127.0.0.1:{port}` (§22.2).
#[cfg(not(target_arch = "wasm32"))]
pub async fn serve_nfs(fs: SurrealFs, port: u16) -> std::io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let server = std::sync::Arc::new(NfsServer::new(fs));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let srv = server.clone();
        tokio::spawn(async move {
            let mut msg_buf = Vec::new();
            loop {
                let mut hdr = [0u8; 4];
                if stream.read_exact(&mut hdr).await.is_err() {
                    break;
                }
                let raw_hdr = u32::from_be_bytes(hdr);
                let last_frag = (raw_hdr & 0x8000_0000) != 0;
                let frag_len = (raw_hdr & 0x7fff_ffff) as usize;
                if frag_len > 1024 * 1024 || msg_buf.len() + frag_len > 1024 * 1024 {
                    break;
                }
                let start = msg_buf.len();
                msg_buf.resize(start + frag_len, 0);
                if stream.read_exact(&mut msg_buf[start..]).await.is_err() {
                    break;
                }
                if last_frag {
                    if let Some(reply) = srv.handle_rpc_call(&msg_buf).await {
                        let out_hdr = 0x8000_0000u32 | (reply.len() as u32);
                        if stream.write_all(&out_hdr.to_be_bytes()).await.is_err() {
                            break;
                        }
                        if stream.write_all(&reply).await.is_err() {
                            break;
                        }
                    }
                    msg_buf.clear();
                }
            }
        });
    }
}

