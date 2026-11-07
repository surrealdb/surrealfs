//! S3-compatible REST API server (§22.3).
//!
//! Enables standard S3 tools, AWS CLI, DuckDB, rclone, and backup tools
//! to read, write, and list objects backed by SurrealDB and FastCDC chunks.

use crate::auth::Authenticator;
use crate::http::{HttpHandler, HttpRequest, HttpResponse};
use crate::serve::{range_not_satisfiable, serve_file, ServeError};
use async_trait::async_trait;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use surrealfs_core::fs::SurrealFs;
use surrealfs_core::paths::normalize_path;
use tokio::sync::RwLock;

/// S3 Multipart upload in progress (§22.3).
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct MultipartUpload {
    bucket: String,
    key: String,
    parts: HashMap<u32, Vec<u8>>,
    total_bytes: usize,
    created_at: chrono::DateTime<Utc>,
}

const MAX_MULTIPART_UPLOADS: usize = 1000;
const MAX_MULTIPART_BYTES: usize = 100 * 1024 * 1024;
const MULTIPART_TTL_SECS: i64 = 3600;

/// S3-compatible API server implementation (§22.3).
pub struct S3Server {
    fs: SurrealFs,
    auth: Authenticator,
    require_auth: bool,
    multipart_uploads: Arc<RwLock<HashMap<String, MultipartUpload>>>,
}

impl S3Server {
    pub fn new(fs: SurrealFs) -> Self {
        Self {
            auth: Authenticator::new(fs.clone()),
            fs,
            require_auth: false,
            multipart_uploads: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_require_auth(mut self, require_auth: bool) -> Self {
        self.require_auth = require_auth;
        self
    }

    /// Splits an S3 request path into (bucket, key).
    fn parse_bucket_and_key(&self, path: &str) -> (Option<String>, Option<String>) {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return (None, None);
        }
        let mut parts = trimmed.splitn(2, '/');
        let bucket = parts.next().map(|b| b.to_string());
        let key = parts.next().map(|k| k.to_string());
        (bucket, key)
    }

    /// Translates (bucket, key) to a SurrealFS filesystem path.
    fn to_sfs_path(&self, bucket: &str, key: Option<&str>) -> String {
        match key {
            Some(k) if !k.is_empty() => normalize_path(&format!("/{}/{}", bucket, k)),
            _ => normalize_path(&format!("/{}", bucket)),
        }
    }

    /// Handles GET / (ListAllMyBuckets).
    async fn handle_list_buckets(&self, fs: &SurrealFs) -> HttpResponse {
        let mut xml = String::from(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ListAllMyBucketsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Owner>
    <ID>surrealfs</ID>
    <DisplayName>surrealfs</DisplayName>
  </Owner>
  <Buckets>"#,
        );

        if let Ok(entries) = fs.ls("/").await {
            for entry in entries {
                if entry.is_folder {
                    let created = entry
                        .created_at
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| Utc::now().to_rfc3339());
                    xml.push_str(&format!(
                        r#"<Bucket><Name>{}</Name><CreationDate>{}</CreationDate></Bucket>"#,
                        escape_xml(&entry.filename),
                        created
                    ));
                }
            }
        }

        xml.push_str("</Buckets></ListAllMyBucketsResult>");
        HttpResponse::ok()
            .header("Content-Type", "application/xml")
            .body_str(&xml)
    }

    /// Traverses all subdirectories in a bucket recursively (§22.3).
    async fn list_bucket_objects_recursive(
        &self,
        fs: &SurrealFs,
        bucket_path: &str,
    ) -> Vec<surrealfs_core::models::FileEntry> {
        let mut all_files = Vec::new();
        let mut queue = vec![bucket_path.to_string()];
        while let Some(dir) = queue.pop() {
            if let Ok(entries) = fs.ls(&dir).await {
                for entry in entries {
                    if entry.is_folder {
                        queue.push(entry.path.clone());
                    } else {
                        all_files.push(entry);
                    }
                }
            }
        }
        all_files
    }

    /// Handles GET /<bucket> (ListObjectsV2).
    async fn handle_list_objects(
        &self,
        fs: &SurrealFs,
        req: &HttpRequest,
        bucket: &str,
    ) -> HttpResponse {
        let bucket_path = format!("/{}", bucket);
        if !fs.exists(&bucket_path).await.unwrap_or(false) {
            return s3_error("NoSuchBucket", "The specified bucket does not exist.", 404);
        }

        let prefix = req.query_param("prefix").unwrap_or("");
        let delimiter = req.query_param("delimiter").unwrap_or("");
        let max_keys: usize = req
            .query_param("max-keys")
            .and_then(|m| m.parse().ok())
            .unwrap_or(1000);

        let files = self.list_bucket_objects_recursive(fs, &bucket_path).await;

        let mut xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Name>{}</Name>
  <Prefix>{}</Prefix>
  <MaxKeys>{}</MaxKeys>
  <IsTruncated>false</IsTruncated>"#,
            escape_xml(bucket),
            escape_xml(prefix),
            max_keys
        );

        let mut count = 0;
        let mut common_prefixes = std::collections::HashSet::new();

        for file in files {
            if file.is_folder {
                continue;
            }
            // Strip bucket prefix to get object key
            let full = &file.path;
            let key = full
                .strip_prefix(&format!("/{}/", bucket))
                .unwrap_or(file.filename.as_str());

            if !prefix.is_empty() && !key.starts_with(prefix) {
                continue;
            }

            if !delimiter.is_empty() {
                let suffix = &key[prefix.len()..];
                if let Some((first, _)) = suffix.split_once(delimiter) {
                    let common = format!("{}{}{}", prefix, first, delimiter);
                    common_prefixes.insert(common);
                    continue;
                }
            }

            if count >= max_keys {
                break;
            }

            let modified = file
                .updated_at
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339());
            let etag = format!("\"gen-{}\"", file.generation);

            xml.push_str("<Contents>");
            xml.push_str(&format!("<Key>{}</Key>", escape_xml(key)));
            xml.push_str(&format!("<LastModified>{}</LastModified>", modified));
            xml.push_str(&format!("<ETag>{}</ETag>", etag));
            xml.push_str(&format!("<Size>{}</Size>", file.size));
            xml.push_str("<StorageClass>STANDARD</StorageClass>");
            xml.push_str("</Contents>");
            count += 1;
        }

        for cp in common_prefixes {
            xml.push_str(&format!(
                "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
                escape_xml(&cp)
            ));
        }

        xml.push_str("</ListBucketResult>");
        HttpResponse::ok()
            .header("Content-Type", "application/xml")
            .body_str(&xml)
    }

    /// Handles GET /<bucket>/<key> (GetObject).
    async fn handle_get_object(
        &self,
        fs: &SurrealFs,
        req: &HttpRequest,
        sfs_path: &str,
    ) -> HttpResponse {
        let stat = match fs.stat(sfs_path).await {
            Ok(Some(s)) => s,
            Ok(None) => return s3_error("NoSuchKey", "The specified key does not exist.", 404),
            Err(e) => return s3_error("InternalError", &e.to_string(), 500),
        };

        if stat.is_folder {
            return s3_error("NoSuchKey", "Object is a folder", 404);
        }

        match serve_file(fs, req, sfs_path, None).await {
            Ok(res) => res,
            Err(ServeError::NotSatisfiable(size)) => range_not_satisfiable(size),
            Err(ServeError::Fs(e)) => s3_error("InternalError", &e.to_string(), 500),
        }
    }

    /// Handles HEAD /<bucket>/<key> (HeadObject).
    async fn handle_head_object(&self, fs: &SurrealFs, sfs_path: &str) -> HttpResponse {
        match fs.stat(sfs_path).await {
            Ok(Some(stat)) if !stat.is_folder => {
                let etag = format!("\"gen-{}\"", stat.generation);
                HttpResponse::ok()
                    .header("Content-Type", &stat.content_type)
                    .header("ETag", &etag)
                    .header("Content-Length", &stat.size.to_string())
                    .header("Accept-Ranges", "bytes")
            }
            _ => HttpResponse::not_found("Not Found"),
        }
    }

    /// Handles PUT /<bucket>/<key> (PutObject).
    async fn handle_put_object(
        &self,
        fs: &SurrealFs,
        req: &HttpRequest,
        sfs_path: &str,
    ) -> HttpResponse {
        // Check for UploadPart query params: ?uploadId=...&partNumber=...
        if let (Some(upload_id), Some(part_num_str)) =
            (req.query_param("uploadId"), req.query_param("partNumber"))
        {
            if let Ok(part_num) = part_num_str.parse::<u32>() {
                return self
                    .handle_upload_part(upload_id, part_num, req.body.clone())
                    .await;
            }
        }

        let written = if req.body_stream.is_some() {
            fs.upload_stream(sfs_path, req.into_body_stream(), None, None)
                .await
        } else {
            fs.write_bytes(sfs_path, &req.body, None).await
        };
        match written {
            Ok(entry) => {
                let etag = format!("\"gen-{}\"", entry.generation);
                HttpResponse::ok()
                    .header("ETag", &etag)
                    .header("Content-Length", "0")
            }
            Err(e) => s3_error("InternalError", &e.to_string(), 500),
        }
    }

    /// Handles DELETE /<bucket>/<key> (DeleteObject).
    async fn handle_delete_object(&self, fs: &SurrealFs, sfs_path: &str) -> HttpResponse {
        let _ = fs.rm(sfs_path, false).await;
        HttpResponse::no_content()
    }

    /// Handles POST requests (InitiateMultipartUpload or CompleteMultipartUpload).
    async fn handle_post(
        &self,
        fs: &SurrealFs,
        req: &HttpRequest,
        bucket: &str,
        key: &str,
        sfs_path: &str,
    ) -> HttpResponse {
        if req.query.contains_key("uploads") {
            // InitiateMultipartUpload
            let now = Utc::now();
            let upload_id = blake3::hash(format!("{}:{}:{}", bucket, key, now).as_bytes())
                .to_hex()
                .to_string();

            let mut map = self.multipart_uploads.write().await;
            // Evict expired multipart uploads older than MULTIPART_TTL_SECS
            map.retain(|_, u| (now - u.created_at).num_seconds() <= MULTIPART_TTL_SECS);
            if map.len() >= MAX_MULTIPART_UPLOADS {
                return s3_error(
                    "SlowDown",
                    "Too many concurrent multipart uploads in progress",
                    503,
                );
            }

            map.insert(
                upload_id.clone(),
                MultipartUpload {
                    bucket: bucket.to_string(),
                    key: key.to_string(),
                    parts: HashMap::new(),
                    total_bytes: 0,
                    created_at: now,
                },
            );

            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<InitiateMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Bucket>{}</Bucket>
  <Key>{}</Key>
  <UploadId>{}</UploadId>
</InitiateMultipartUploadResult>"#,
                escape_xml(bucket),
                escape_xml(key),
                upload_id
            );
            return HttpResponse::ok()
                .header("Content-Type", "application/xml")
                .body_str(&xml);
        }

        if let Some(upload_id) = req.query_param("uploadId") {
            // CompleteMultipartUpload
            let upload = {
                let mut map = self.multipart_uploads.write().await;
                map.remove(upload_id)
            };

            let Some(upload) = upload else {
                return s3_error(
                    "NoSuchUpload",
                    "The specified multipart upload does not exist.",
                    404,
                );
            };

            // Assemble parts in sorted part order
            let mut part_indices: Vec<u32> = upload.parts.keys().copied().collect();
            part_indices.sort_unstable();

            let mut full_body = Vec::new();
            for idx in part_indices {
                if let Some(bytes) = upload.parts.get(&idx) {
                    full_body.extend_from_slice(bytes);
                }
            }

            match fs.write_bytes(sfs_path, &full_body, None).await {
                Ok(entry) => {
                    let etag = format!("\"gen-{}\"", entry.generation);
                    let xml = format!(
                        r#"<?xml version="1.0" encoding="UTF-8"?>
<CompleteMultipartUploadResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <Location>/{}/{}</Location>
  <Bucket>{}</Bucket>
  <Key>{}</Key>
  <ETag>{}</ETag>
</CompleteMultipartUploadResult>"#,
                        escape_xml(bucket),
                        escape_xml(key),
                        escape_xml(bucket),
                        escape_xml(key),
                        etag
                    );
                    HttpResponse::ok()
                        .header("Content-Type", "application/xml")
                        .body_str(&xml)
                }
                Err(e) => s3_error("InternalError", &e.to_string(), 500),
            }
        } else {
            HttpResponse::bad_request("Invalid POST request")
        }
    }

    /// Handles UploadPart for multipart uploads (§22.3).
    async fn handle_upload_part(
        &self,
        upload_id: &str,
        part_number: u32,
        data: Vec<u8>,
    ) -> HttpResponse {
        let mut map = self.multipart_uploads.write().await;
        if let Some(upload) = map.get_mut(upload_id) {
            let prev_len = upload.parts.get(&part_number).map(|p| p.len()).unwrap_or(0);
            let new_total = upload.total_bytes.saturating_sub(prev_len) + data.len();
            if new_total > MAX_MULTIPART_BYTES {
                return s3_error(
                    "EntityTooLarge",
                    "Multipart upload exceeds maximum total size",
                    400,
                );
            }
            let part_etag = format!("\"part-{}\"", blake3::hash(&data).to_hex());
            upload.total_bytes = new_total;
            upload.parts.insert(part_number, data);
            HttpResponse::ok().header("ETag", &part_etag)
        } else {
            s3_error("NoSuchUpload", "Upload does not exist", 404)
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl HttpHandler for S3Server {
    async fn handle(&self, req: HttpRequest) -> HttpResponse {
        // Authenticate S3 request and scope caller identity (§22)
        let fs = if let Some(auth_hdr) = req.header("authorization") {
            match self.auth.authenticate_s3(Some(auth_hdr)).await {
                Ok(user) => self.fs.with_caller(&user.user_id),
                Err(_) => return s3_error("AccessDenied", "Access Denied", 403),
            }
        } else if self.require_auth {
            return s3_error("AccessDenied", "Access Denied", 403);
        } else {
            self.fs.clone()
        };

        let (bucket, key) = self.parse_bucket_and_key(&req.path);

        match (bucket, key) {
            (None, None) => match req.method.as_str() {
                "GET" => self.handle_list_buckets(&fs).await,
                _ => HttpResponse::method_not_allowed(),
            },
            (Some(b), None) => match req.method.as_str() {
                "GET" => self.handle_list_objects(&fs, &req, &b).await,
                "PUT" => {
                    // CreateBucket
                    let sfs_path = format!("/{}", b);
                    match fs.mkdir(&sfs_path, false).await {
                        Ok(_) => HttpResponse::ok().header("Location", &format!("/{}", b)),
                        Err(_) => HttpResponse::ok(),
                    }
                }
                "DELETE" => {
                    // DeleteBucket
                    let sfs_path = format!("/{}", b);
                    let _ = fs.rm(&sfs_path, false).await;
                    HttpResponse::no_content()
                }
                _ => HttpResponse::method_not_allowed(),
            },
            (Some(b), Some(k)) => {
                let sfs_path = self.to_sfs_path(&b, Some(&k));
                match req.method.as_str() {
                    "GET" => self.handle_get_object(&fs, &req, &sfs_path).await,
                    "HEAD" => self.handle_head_object(&fs, &sfs_path).await,
                    "PUT" => self.handle_put_object(&fs, &req, &sfs_path).await,
                    "DELETE" => {
                        // Check if AbortMultipartUpload: ?uploadId=...
                        if let Some(upload_id) = req.query_param("uploadId") {
                            let mut map = self.multipart_uploads.write().await;
                            map.remove(upload_id);
                            HttpResponse::no_content()
                        } else {
                            self.handle_delete_object(&fs, &sfs_path).await
                        }
                    }
                    "POST" => self.handle_post(&fs, &req, &b, &k, &sfs_path).await,
                    _ => HttpResponse::method_not_allowed(),
                }
            }
            _ => HttpResponse::bad_request("Invalid S3 URI"),
        }
    }
}

/// Formats a standard AWS S3 XML error response.
fn s3_error(code: &str, message: &str, status: u16) -> HttpResponse {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Error>
  <Code>{}</Code>
  <Message>{}</Message>
  <Resource>surrealfs</Resource>
  <RequestId>{}</RequestId>
</Error>"#,
        code,
        escape_xml(message),
        Utc::now().timestamp_millis()
    );

    HttpResponse::new(status, code)
        .header("Content-Type", "application/xml")
        .body_str(&xml)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
