//! Object storage for chunk bytes (docs/blob-storage.md).
//!
//! A [`BlobStore`] holds immutable, content-addressed chunk bodies. A body is at most one chunk, so
//! it is passed whole; a file larger than a chunk is moved chunk by chunk. SurrealDB owns every
//! decision about who may read a key: a store is handed only keys taken from a manifest the
//! database returned for the caller.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::errors::{Result, SurrealFsError};

/// Bound of every store: shared across the threads of a native runtime, and `Send + Sync` on
/// `wasm32` too (a store wrapping JavaScript handles asserts it, as the target is single-threaded).
pub trait Shared: Send + Sync {}
impl<T: Send + Sync> Shared for T {}

/// What a store knows about one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInfo {
    pub key: String,
    pub size: u64,
    /// Seconds since the Unix epoch of the last write of the key, when the store reports one.
    pub modified: Option<i64>,
}

/// The key of the object holding the chunk named `chunk_id` (`b3:<hex>`).
pub fn blob_key(chunk_id: &str) -> String {
    let hash = chunk_id.strip_prefix("b3:").unwrap_or(chunk_id);
    let shard = hash.get(..2).unwrap_or("00");
    format!("blobs/{shard}/{hash}")
}

/// Chunk-body storage.
///
/// Contract:
/// - `put` stores `data` at `key`, replacing any object there. Rewriting a key refreshes its
///   modification time, which is what protects a chunk that is re-uploaded while the sweep runs.
/// - `get` returns the bytes of `range` (clamped to the object), or the whole object. A missing key
///   is [`SurrealFsError::NotFound`].
/// - `head` returns `None` for a missing key.
/// - `delete` of a missing key succeeds.
/// - `list` returns every object whose key starts with `prefix`, in any order.
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait BlobStore: Shared {
    async fn put(&self, key: &str, data: Vec<u8>) -> Result<()>;
    async fn get(&self, key: &str, range: Option<Range<u64>>) -> Result<Vec<u8>>;
    async fn head(&self, key: &str) -> Result<Option<BlobInfo>>;
    async fn delete(&self, key: &str) -> Result<()>;
    async fn list(&self, prefix: &str) -> Result<Vec<BlobInfo>>;
}

pub type SharedBlobStore = Arc<dyn BlobStore>;

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn clamp(range: Option<Range<u64>>, len: usize) -> Range<usize> {
    match range {
        None => 0..len,
        Some(r) => {
            let start = (r.start as usize).min(len);
            let end = (r.end as usize).min(len).max(start);
            start..end
        }
    }
}

/// A store held in memory: tests, and deployments with no object store.
#[derive(Default)]
pub struct MemoryBlobStore {
    objects: Mutex<HashMap<String, (Vec<u8>, i64)>>,
}

impl MemoryBlobStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of objects held.
    pub fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl BlobStore for MemoryBlobStore {
    async fn put(&self, key: &str, data: Vec<u8>) -> Result<()> {
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), (data, now_secs()));
        Ok(())
    }

    async fn get(&self, key: &str, range: Option<Range<u64>>) -> Result<Vec<u8>> {
        let objects = self.objects.lock().unwrap();
        let (data, _) = objects
            .get(key)
            .ok_or_else(|| SurrealFsError::NotFound(format!("blob {key}")))?;
        Ok(data[clamp(range, data.len())].to_vec())
    }

    async fn head(&self, key: &str) -> Result<Option<BlobInfo>> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(key)
            .map(|(d, m)| BlobInfo {
                key: key.to_string(),
                size: d.len() as u64,
                modified: Some(*m),
            }))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.objects.lock().unwrap().remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobInfo>> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, (d, m))| BlobInfo {
                key: k.clone(),
                size: d.len() as u64,
                modified: Some(*m),
            })
            .collect())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use futures::TryStreamExt;
    use object_store::path::Path;
    use object_store::{GetOptions, ObjectStore, ObjectStoreExt, PutPayload};

    fn storage(e: object_store::Error) -> SurrealFsError {
        match e {
            object_store::Error::NotFound { path, .. } => {
                SurrealFsError::NotFound(format!("blob {path}"))
            }
            other => SurrealFsError::Storage(other.to_string()),
        }
    }

    /// A [`BlobStore`] over any `object_store` backend: a local directory, S3, GCS or Azure.
    pub struct ObjectStoreBlobStore {
        inner: Arc<dyn ObjectStore>,
    }

    impl ObjectStoreBlobStore {
        pub fn new(inner: Arc<dyn ObjectStore>) -> Self {
            Self { inner }
        }

        /// A store rooted at the directory `path`, created when absent.
        pub fn directory(path: &std::path::Path) -> Result<Self> {
            std::fs::create_dir_all(path)
                .map_err(|e| SurrealFsError::Io(format!("create {}: {e}", path.display())))?;
            let fs =
                object_store::local::LocalFileSystem::new_with_prefix(path).map_err(storage)?;
            Ok(Self::new(Arc::new(fs)))
        }
    }

    fn info(meta: object_store::ObjectMeta) -> BlobInfo {
        BlobInfo {
            key: meta.location.to_string(),
            size: meta.size,
            modified: Some(meta.last_modified.timestamp()),
        }
    }

    #[async_trait]
    impl BlobStore for ObjectStoreBlobStore {
        async fn put(&self, key: &str, data: Vec<u8>) -> Result<()> {
            self.inner
                .put(&Path::from(key), PutPayload::from(data))
                .await
                .map(|_| ())
                .map_err(storage)
        }

        async fn get(&self, key: &str, range: Option<Range<u64>>) -> Result<Vec<u8>> {
            let path = Path::from(key);
            let bytes = match range {
                None => self.inner.get(&path).await.map_err(storage)?.bytes().await,
                Some(r) => {
                    let size = self.inner.head(&path).await.map_err(storage)?.size;
                    let end = r.end.min(size);
                    if r.start >= end {
                        return Ok(Vec::new());
                    }
                    let opts = GetOptions {
                        range: Some((r.start..end).into()),
                        ..Default::default()
                    };
                    self.inner
                        .get_opts(&path, opts)
                        .await
                        .map_err(storage)?
                        .bytes()
                        .await
                }
            };
            Ok(bytes.map_err(storage)?.to_vec())
        }

        async fn head(&self, key: &str) -> Result<Option<BlobInfo>> {
            match self.inner.head(&Path::from(key)).await {
                Ok(meta) => Ok(Some(info(meta))),
                Err(object_store::Error::NotFound { .. }) => Ok(None),
                Err(e) => Err(storage(e)),
            }
        }

        async fn delete(&self, key: &str) -> Result<()> {
            match self.inner.delete(&Path::from(key)).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
                Err(e) => Err(storage(e)),
            }
        }

        async fn list(&self, prefix: &str) -> Result<Vec<BlobInfo>> {
            let prefix = Path::from(prefix.trim_end_matches('/'));
            self.inner
                .list(Some(&prefix))
                .map_ok(info)
                .try_collect()
                .await
                .map_err(storage)
        }
    }

    /// The blob store a configuration string names.
    ///
    /// `memory`, `dir:<path>` or a bare filesystem path, and with the `cloud-stores` feature
    /// `s3://`, `gs://` and `az://` URLs, whose credentials come from the environment.
    pub fn blob_store_from_url(spec: &str) -> Result<SharedBlobStore> {
        let spec = spec.trim();
        if spec == "memory" {
            return Ok(Arc::new(MemoryBlobStore::new()));
        }
        #[cfg(feature = "cloud-stores")]
        {
            use object_store::aws::AmazonS3Builder;
            use object_store::azure::MicrosoftAzureBuilder;
            use object_store::gcp::GoogleCloudStorageBuilder;
            let cloud = |e: object_store::Error| SurrealFsError::Storage(e.to_string());
            if spec.starts_with("s3://") {
                let store = AmazonS3Builder::from_env()
                    .with_url(spec)
                    .build()
                    .map_err(cloud)?;
                return Ok(Arc::new(ObjectStoreBlobStore::new(Arc::new(store))));
            }
            if spec.starts_with("gs://") {
                let store = GoogleCloudStorageBuilder::from_env()
                    .with_url(spec)
                    .build()
                    .map_err(cloud)?;
                return Ok(Arc::new(ObjectStoreBlobStore::new(Arc::new(store))));
            }
            if spec.starts_with("az://") || spec.starts_with("azure://") {
                let store = MicrosoftAzureBuilder::from_env()
                    .with_url(spec)
                    .build()
                    .map_err(cloud)?;
                return Ok(Arc::new(ObjectStoreBlobStore::new(Arc::new(store))));
            }
        }
        let path = spec.strip_prefix("dir:").unwrap_or(spec);
        if path.contains("://") {
            return Err(SurrealFsError::InvalidArguments(format!(
                "unsupported blob store `{spec}`"
            )));
        }
        Ok(Arc::new(ObjectStoreBlobStore::directory(
            std::path::Path::new(path),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn exercise(store: &dyn BlobStore) {
        let key = blob_key("b3:abcdef");
        assert_eq!(key, "blobs/ab/abcdef");
        assert!(store.head(&key).await.unwrap().is_none());
        assert!(matches!(
            store.get(&key, None).await,
            Err(SurrealFsError::NotFound(_))
        ));
        store.put(&key, b"0123456789".to_vec()).await.unwrap();
        assert_eq!(store.get(&key, None).await.unwrap(), b"0123456789");
        assert_eq!(store.get(&key, Some(2..5)).await.unwrap(), b"234");
        assert_eq!(store.get(&key, Some(8..99)).await.unwrap(), b"89");
        assert!(store.get(&key, Some(20..30)).await.unwrap().is_empty());
        let head = store.head(&key).await.unwrap().unwrap();
        assert_eq!(head.size, 10);
        assert_eq!(store.list("blobs/").await.unwrap().len(), 1);
        assert!(store.list("other/").await.unwrap().is_empty());
        store.put(&key, b"abc".to_vec()).await.unwrap();
        assert_eq!(store.get(&key, None).await.unwrap(), b"abc");
        store.delete(&key).await.unwrap();
        store.delete(&key).await.unwrap();
        assert!(store.head(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn memory_store_contract() {
        exercise(&MemoryBlobStore::new()).await;
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn directory_store_contract() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStoreBlobStore::directory(dir.path()).unwrap();
        exercise(&store).await;
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn urls_select_stores() {
        assert!(blob_store_from_url("memory").is_ok());
        let dir = tempfile::tempdir().unwrap();
        assert!(blob_store_from_url(&format!("dir:{}", dir.path().display())).is_ok());
        assert!(blob_store_from_url("ftp://nope").is_err());
    }
}
