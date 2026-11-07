//! Streamed reads and uploads of file content (docs/blob-storage.md).
//!
//! A read asks SurrealDB for the manifest of the bytes it needs, a page at a time, and fetches the
//! covering chunks one by one, so memory is bounded by a chunk. An upload cuts the incoming bytes
//! into chunks as they arrive and stores each before reading on.

use std::ops::Range;
use std::pin::Pin;

use futures::stream::{self, Stream, StreamExt, TryStreamExt};
use serde_json::Value;

use crate::chunking::{decompress_chunk, FastCdcConfig, StreamChunker};
use crate::errors::{Result, SurrealFsError};
use crate::fs::{extract_bytes, SurrealFs};
use crate::models::FileEntry;
use crate::normalize_path;

#[cfg(not(target_arch = "wasm32"))]
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>>> + Send>>;
#[cfg(target_arch = "wasm32")]
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>>>>>;

/// Files up to this size are written inline, in the `file` row.
pub const INLINE_MAX_BYTES: usize = 1024 * 1024;

/// How many bytes of manifest one request to the database covers.
const MANIFEST_PAGE_BYTES: u64 = 64 * 1024 * 1024;

/// A requested slice of a file, in the forms an HTTP `Range` header takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    Full,
    /// `bytes=start-end`, both ends inclusive.
    Bounded(u64, u64),
    /// `bytes=start-`.
    From(u64),
    /// `bytes=-n`, the last `n` bytes.
    Suffix(u64),
}

impl ByteRange {
    /// The half-open range this selects of a file of `size` bytes, or `None` when no byte of the
    /// file is selected (HTTP `416`). A full read of an empty file selects `0..0`.
    pub fn resolve(&self, size: u64) -> Option<Range<u64>> {
        match *self {
            ByteRange::Full => Some(0..size),
            ByteRange::Bounded(start, end) if start <= end && start < size => {
                Some(start..end.saturating_add(1).min(size))
            }
            ByteRange::From(start) if start < size => Some(start..size),
            ByteRange::Suffix(n) if n > 0 && size > 0 => Some(size.saturating_sub(n)..size),
            _ => None,
        }
    }
}

/// A file opened for reading.
pub struct FileStream {
    /// The size of the whole file.
    pub size: u64,
    pub generation: u64,
    pub content_type: String,
    /// The bytes `body` yields.
    pub range: Range<u64>,
    pub body: ByteStream,
}

struct ChunkRef {
    offset: u64,
    length: u64,
    codec: String,
    tier: String,
    location: Option<String>,
    bytes: Option<Vec<u8>>,
}

struct Manifest {
    chunked: bool,
    size: u64,
    generation: u64,
    content_type: String,
    chunks: Vec<ChunkRef>,
    data: Option<Vec<u8>>,
}

impl SurrealFs {
    async fn manifest(&self, path: &str, offset: u64, length: u64) -> Result<Manifest> {
        let mut res = self
            .db
            .query("RETURN fn::sfs_read_manifest($path, $offset, $length, $caller);")
            .bind(("path", path.to_string()))
            .bind(("offset", offset as i64))
            .bind(("length", length.min(i64::MAX as u64) as i64))
            .bind(("caller", self.caller.clone()))
            .await?;
        let val: Option<Value> = res.take(0usize)?;
        let v = val.ok_or_else(|| SurrealFsError::NotFound(format!("File not found: {path}")))?;
        let chunks = v
            .get("chunks")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|c| ChunkRef {
                        offset: c.get("offset").and_then(|x| x.as_u64()).unwrap_or(0),
                        length: c.get("length").and_then(|x| x.as_u64()).unwrap_or(0),
                        codec: c
                            .get("codec")
                            .and_then(|x| x.as_str())
                            .unwrap_or("none")
                            .to_string(),
                        tier: c
                            .get("tier")
                            .and_then(|x| x.as_str())
                            .unwrap_or("inline")
                            .to_string(),
                        location: c.get("location").and_then(|x| x.as_str()).map(String::from),
                        bytes: extract_bytes(c.get("bytes")),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let data = match v.get("data") {
            Some(Value::String(s)) => Some(s.as_bytes().to_vec()),
            Some(other) if !other.is_null() => extract_bytes(Some(other)),
            _ => None,
        };
        Ok(Manifest {
            chunked: v.get("chunked").and_then(|x| x.as_bool()).unwrap_or(false),
            size: v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            generation: v.get("generation").and_then(|x| x.as_u64()).unwrap_or(0),
            content_type: v
                .get("content_type")
                .and_then(|x| x.as_str())
                .unwrap_or("application/octet-stream")
                .to_string(),
            chunks,
            data,
        })
    }

    async fn chunk_bytes(&self, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let raw = if chunk.tier == "object" {
            let location = chunk.location.as_deref().ok_or_else(|| {
                SurrealFsError::Database("Object chunk without a location".to_string())
            })?;
            let store = self.blobs.as_ref().ok_or_else(|| {
                SurrealFsError::Storage(
                    "File content is in an object store, but none is configured".to_string(),
                )
            })?;
            store.get(location, None).await?
        } else {
            chunk.bytes.clone().unwrap_or_default()
        };
        decompress_chunk(&raw, &chunk.codec)
            .map_err(|e| SurrealFsError::Other(format!("Failed to decompress chunk: {e}")))
    }

    /// Open `path` for reading the bytes `range` selects.
    ///
    /// Read permission is checked, and the file's size, generation and type are read, before this
    /// returns. A range that selects no byte is [`SurrealFsError::InvalidArguments`], which an
    /// HTTP caller answers with `416` and the file's size. The body then pulls the manifest a
    /// page at a time and one chunk at a time; if the file changes while it streams, the body
    /// ends with a conflict error rather than mixing two versions.
    pub async fn read_stream(&self, path: &str, range: ByteRange) -> Result<FileStream> {
        let norm = normalize_path(path);
        let probe = self.manifest(&norm, 0, 0).await?;
        let Some(selected) = range.resolve(probe.size) else {
            return Err(SurrealFsError::InvalidArguments(format!(
                "range not satisfiable for a file of {} bytes",
                probe.size
            )));
        };

        if !probe.chunked {
            let whole = self
                .manifest(&norm, 0, u64::MAX)
                .await?
                .data
                .unwrap_or_default();
            let start = (selected.start as usize).min(whole.len());
            let end = (selected.end as usize).min(whole.len()).max(start);
            let piece = whole[start..end].to_vec();
            return Ok(FileStream {
                size: probe.size,
                generation: probe.generation,
                content_type: probe.content_type,
                range: selected,
                body: Box::pin(stream::once(async move { Ok(piece) })),
            });
        }

        struct State {
            fs: SurrealFs,
            path: String,
            generation: u64,
            next: u64,
            end: u64,
            pending: std::collections::VecDeque<ChunkRef>,
        }
        let state = State {
            fs: self.clone(),
            path: norm,
            generation: probe.generation,
            next: selected.start,
            end: selected.end,
            pending: Default::default(),
        };
        let body = stream::try_unfold(state, |mut st| async move {
            loop {
                if let Some(chunk) = st.pending.pop_front() {
                    let from = st.next.max(chunk.offset);
                    let to = st.end.min(chunk.offset + chunk.length);
                    st.next = chunk.offset + chunk.length;
                    if from >= to {
                        continue;
                    }
                    let bytes = st.fs.chunk_bytes(&chunk).await?;
                    let lo = (from - chunk.offset) as usize;
                    let hi = ((to - chunk.offset) as usize).min(bytes.len());
                    return Ok(Some((bytes[lo.min(hi)..hi].to_vec(), st)));
                }
                if st.next >= st.end {
                    return Ok(None);
                }
                let page = (st.end - st.next).min(MANIFEST_PAGE_BYTES);
                let m = st.fs.manifest(&st.path, st.next, page).await?;
                if m.generation != st.generation || !m.chunked {
                    return Err(SurrealFsError::Conflict(format!(
                        "{} changed while it was being read",
                        st.path
                    )));
                }
                if m.chunks.is_empty() {
                    return Err(SurrealFsError::Database(format!(
                        "manifest of {} has a gap at byte {}",
                        st.path, st.next
                    )));
                }
                st.pending = m.chunks.into();
            }
        });
        Ok(FileStream {
            size: probe.size,
            generation: probe.generation,
            content_type: probe.content_type,
            range: selected,
            body: Box::pin(body),
        })
    }

    /// Write `body` to `path`: inline when it is at most [`INLINE_MAX_BYTES`] long, else in
    /// chunks stored as they are cut, so memory is bounded by a chunk and not by the file.
    pub async fn upload_stream(
        &self,
        path: &str,
        mut body: ByteStream,
        if_generation: Option<u64>,
        config: Option<FastCdcConfig>,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        let mut head: Vec<u8> = Vec::new();
        let mut rest: Option<Vec<u8>> = None;
        while let Some(piece) = body.try_next().await? {
            head.extend_from_slice(&piece);
            if head.len() > INLINE_MAX_BYTES {
                rest = Some(Vec::new());
                break;
            }
        }
        if rest.is_none() {
            return self.write_bytes(&norm, &head, if_generation).await;
        }

        let mut res = self
            .db
            .query("RETURN fn::sfs_upload_begin($path, 0, [], $caller);")
            .bind(("path", norm.clone()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let begun: Option<Value> = res.take(0usize)?;
        let upload_id = begun
            .as_ref()
            .and_then(|v| v.get("upload_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| SurrealFsError::Database("Empty upload session".to_string()))?
            .to_string();

        let mut chunker = StreamChunker::new(config);
        let mut offsets: Vec<i64> = Vec::new();
        let mut lengths: Vec<i64> = Vec::new();
        let mut ready = chunker.push(&head);
        drop(head);
        loop {
            for chunk in ready.drain(..) {
                offsets.push(chunk.offset as i64);
                lengths.push(chunk.length as i64);
                self.store_part(&upload_id, &chunk).await?;
            }
            match body.try_next().await? {
                Some(piece) => ready = chunker.push(&piece),
                None => {
                    ready = chunker.finish();
                    if ready.is_empty() {
                        break;
                    }
                    for chunk in ready.drain(..) {
                        offsets.push(chunk.offset as i64);
                        lengths.push(chunk.length as i64);
                        self.store_part(&upload_id, &chunk).await?;
                    }
                    break;
                }
            }
        }

        let mut res = self
            .db
            .query("RETURN fn::sfs_upload_commit($upload_id, $if_generation, $offsets, $lengths, $caller);")
            .bind(("upload_id", upload_id))
            .bind(("if_generation", if_generation.map(|g| g as i64)))
            .bind(("offsets", offsets))
            .bind(("lengths", lengths))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<Value> = res.take(0usize)?;
        let v = entry.ok_or_else(|| {
            SurrealFsError::Database("Empty response from fn::sfs_upload_commit".to_string())
        })?;
        serde_json::from_value(v).map_err(|e| SurrealFsError::Database(e.to_string()))
    }

    /// Store one chunk of a streamed upload: its bytes first, unless the chunk is already held,
    /// then its row and its place in the session.
    async fn store_part(&self, upload_id: &str, chunk: &crate::models::BlobChunk) -> Result<()> {
        let data = chunk.data.as_ref().expect("chunks carry their bytes");
        let mut held = self
            .db
            .query("RETURN fn::sfs_upload_have($chunk_id);")
            .bind(("chunk_id", chunk.chunk_id.clone()))
            .await?
            .take::<Option<bool>>(0usize)?
            .unwrap_or(false);
        loop {
            let location = match (&self.blobs, held) {
                (Some(store), false) => {
                    let key = crate::blob::blob_key(&chunk.chunk_id);
                    store.put(&key, data.clone()).await?;
                    Some(key)
                }
                (Some(_), true) => Some(crate::blob::blob_key(&chunk.chunk_id)),
                (None, _) => None,
            };
            let send_bytes = !held && location.is_none();
            let sql = format!(
                "RETURN fn::sfs_upload_part($upload_id, $chunk_id, $uncompressed_size, $stored_size, $codec, {}, {}, $caller);",
                if send_bytes { "<bytes>$data" } else { "NONE" },
                if location.is_some() { "$location" } else { "NONE" },
            );
            let mut q = self
                .db
                .query(&sql)
                .bind(("upload_id", upload_id.to_string()))
                .bind(("chunk_id", chunk.chunk_id.clone()))
                .bind(("uncompressed_size", chunk.uncompressed_size as i64))
                .bind(("stored_size", chunk.stored_size as i64))
                .bind(("codec", chunk.codec.clone()))
                .bind(("caller", self.caller.clone()));
            if let Some(key) = location {
                q = q.bind(("location", key));
            }
            if send_bytes {
                q = q.bind(("data", data.clone()));
            }
            match q.await {
                Ok(r) => {
                    r.check()?;
                    return Ok(());
                }
                // The blob row was collected between the check and the append: send the bytes.
                Err(e) if held && e.to_string().contains("missing_chunk") => held = false,
                Err(e) => return Err(e),
            }
        }
    }

    /// Read `length` bytes of `path` from `offset`, clamped to the file.
    pub async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let range = ByteRange::Bounded(offset, offset.saturating_add(length - 1));
        let file = match self.read_stream(path, range).await {
            Ok(f) => f,
            Err(SurrealFsError::InvalidArguments(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::with_capacity((file.range.end - file.range.start) as usize);
        let mut body = file.body;
        while let Some(piece) = body.next().await {
            out.extend_from_slice(&piece?);
        }
        Ok(out)
    }
}
