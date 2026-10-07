//! WebDAV protocol server implementation (§22.1).
//!
//! Enables native mounting on macOS Finder, Windows Explorer, GNOME,
//! KDE, and iOS/iPadOS Files without kernel extensions or macFUSE.

use crate::auth::Authenticator;
use crate::http::{HttpHandler, HttpRequest, HttpResponse};
use async_trait::async_trait;
use chrono::Utc;
use surrealfs_core::errors::SurrealFsError;
use surrealfs_core::fs::SurrealFs;
use surrealfs_core::models::FileEntry;
use surrealfs_core::paths::normalize_path;

/// WebDAV server implementation (§22.1).
pub struct WebDavServer {
    fs: SurrealFs,
    auth: Authenticator,
    prefix: String,
}

impl WebDavServer {
    pub fn new(fs: SurrealFs, prefix: &str) -> Self {
        let clean_prefix = if prefix.is_empty() || prefix == "/" {
            String::new()
        } else {
            format!("/{}", prefix.trim_matches('/'))
        };

        Self {
            auth: Authenticator::new(fs.clone()),
            fs,
            prefix: clean_prefix,
        }
    }

    /// Strips the server URL prefix and returns the normalized SurrealFS path.
    fn resolve_path(&self, req_path: &str) -> String {
        let trimmed = if !self.prefix.is_empty() && req_path.starts_with(&self.prefix) {
            &req_path[self.prefix.len()..]
        } else {
            req_path
        };
        normalize_path(trimmed)
    }

    /// Builds a public href including the prefix.
    fn make_href(&self, sfs_path: &str, is_dir: bool) -> String {
        let clean_sfs = if sfs_path == "/" { "" } else { sfs_path };
        let mut href = format!("{}{}", self.prefix, clean_sfs);
        if is_dir && !href.ends_with('/') {
            href.push('/');
        }
        if href.is_empty() {
            href.push('/');
        }
        href
    }

    /// Handles OPTIONS requests (advertises DAV Class 1 and Class 2 compliance).
    fn handle_options(&self) -> HttpResponse {
        HttpResponse::ok()
            .header("DAV", "1, 2")
            .header("MS-Author-Via", "DAV")
            .header(
                "Allow",
                "OPTIONS, GET, HEAD, POST, PUT, DELETE, PROPFIND, PROPPATCH, MKCOL, COPY, MOVE, LOCK, UNLOCK",
            )
    }

    /// Handles PROPFIND requests (returns XML Multi-Status response).
    async fn handle_propfind(&self, req: &HttpRequest, sfs_path: &str) -> HttpResponse {
        // macOS Finder / Windows Explorer quirk: ignore AppleDouble metadata files
        if is_macos_appledouble(sfs_path) {
            return HttpResponse::not_found("AppleDouble file ignored");
        }

        let stat = match self.fs.stat(sfs_path).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                // If checking root folder, synthesize root entry
                if sfs_path == "/" {
                    FileEntry {
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
                    }
                } else {
                    return HttpResponse::not_found("Resource not found");
                }
            }
            Err(e) => return HttpResponse::internal_error(&e.to_string()),
        };

        let depth = req.header("depth").unwrap_or("1");

        let mut entries = vec![stat.clone()];
        if stat.is_folder && depth != "0" {
            if let Ok(children) = self.fs.ls(sfs_path).await {
                for child in children {
                    // Filter out macOS AppleDouble clutter from visible folder listings
                    if !is_macos_appledouble(&child.filename) {
                        entries.push(child);
                    }
                }
            }
        }

        let mut xml = String::from(r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">"#);
        for entry in entries {
            let href = self.make_href(&entry.path, entry.is_folder);
            let display_name = if entry.path == "/" {
                "SurrealFS"
            } else {
                &entry.filename
            };

            let created_iso = entry
                .created_at
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339());
            let modified_rfc1123 = entry
                .updated_at
                .map(|t| t.to_rfc2822())
                .unwrap_or_else(|| Utc::now().to_rfc2822());

            xml.push_str("<D:response>");
            xml.push_str(&format!("<D:href>{}</D:href>", href));
            xml.push_str("<D:propstat><D:prop>");
            xml.push_str(&format!("<D:displayname>{}</D:displayname>", escape_xml(display_name)));
            xml.push_str(&format!("<D:creationdate>{}</D:creationdate>", created_iso));
            xml.push_str(&format!("<D:getlastmodified>{}</D:getlastmodified>", modified_rfc1123));

            if entry.is_folder {
                xml.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
            } else {
                xml.push_str("<D:resourcetype/>");
                xml.push_str(&format!("<D:getcontentlength>{}</D:getcontentlength>", entry.size));
                xml.push_str(&format!(
                    "<D:getcontenttype>{}</D:getcontenttype>",
                    escape_xml(&entry.content_type)
                ));
                let etag = format!("\"gen-{}\"", entry.generation);
                xml.push_str(&format!("<D:getetag>{}</D:getetag>", etag));
            }

            // Supported lock properties (DAV Class 2)
            xml.push_str(r#"<D:supportedlock>
                <D:lockentry>
                    <D:lockscope><D:exclusive/></D:lockscope>
                    <D:locktype><D:write/></D:locktype>
                </D:lockentry>
            </D:supportedlock>"#);

            xml.push_str("</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>");
            xml.push_str("</D:response>");
        }
        xml.push_str("</D:multistatus>");

        HttpResponse::multi_status()
            .header("Content-Type", "application/xml; charset=utf-8")
            .body_str(&xml)
    }

    /// Handles GET and HEAD requests with Range header support (§19.4, §22.1).
    async fn handle_get(&self, req: &HttpRequest, sfs_path: &str, is_head: bool) -> HttpResponse {
        let stat = match self.fs.stat(sfs_path).await {
            Ok(Some(s)) => s,
            Ok(None) => return HttpResponse::not_found("File not found"),
            Err(e) => return HttpResponse::internal_error(&e.to_string()),
        };

        if stat.is_folder {
            // Render HTML directory index for browsers
            if is_head {
                return HttpResponse::ok().header("Content-Type", "text/html; charset=utf-8");
            }
            let children = self.fs.ls(sfs_path).await.unwrap_or_default();
            let mut html = format!("<!DOCTYPE html><html><head><title>Index of {}</title></head><body><h1>Index of {}</h1><hr><ul>", sfs_path, sfs_path);
            if sfs_path != "/" {
                html.push_str("<li><a href=\"..\">..</a></li>");
            }
            for child in children {
                let name = if child.is_folder {
                    format!("{}/", child.filename)
                } else {
                    child.filename
                };
                html.push_str(&format!("<li><a href=\"{}\">{}</a></li>", name, name));
            }
            html.push_str("</ul><hr><i>SurrealFS WebDAV Server</i></body></html>");
            return HttpResponse::ok()
                .header("Content-Type", "text/html; charset=utf-8")
                .body_str(&html);
        }

        let etag = format!("\"gen-{}\"", stat.generation);

        // Check If-None-Match for cache revalidation
        if let Some(if_none_match) = req.header("if-none-match") {
            if if_none_match == "*" || if_none_match == etag {
                return HttpResponse::not_modified().header("ETag", &etag);
            }
        }

        // Handle Range requests (§19.4)
        if let Some(range_header) = req.header("range") {
            if let Some(range) = parse_byte_range(range_header, stat.size) {
                let (start, end) = range;
                let len = end - start + 1;
                let data = match self.fs.read_range(sfs_path, start, len).await {
                    Ok(d) => d,
                    Err(_) => {
                        // Fallback to full read and slice
                        match self.fs.read_bytes(sfs_path).await {
                            Ok(all) => {
                                let s = (start as usize).min(all.len());
                                let e = ((end + 1) as usize).min(all.len());
                                all[s..e].to_vec()
                            }
                            Err(e) => return HttpResponse::internal_error(&e.to_string()),
                        }
                    }
                };

                let mut res = HttpResponse::partial_content()
                    .header("Content-Type", &stat.content_type)
                    .header("ETag", &etag)
                    .header("Accept-Ranges", "bytes")
                    .header(
                        "Content-Range",
                        &format!("bytes {}-{}/{}", start, end, stat.size),
                    );

                if !is_head {
                    res = res.body(data);
                }
                return res;
            }
        }

        // Full content GET
        let data = if is_head {
            Vec::new()
        } else {
            match self.fs.read_bytes(sfs_path).await {
                Ok(d) => d,
                Err(e) => return HttpResponse::internal_error(&e.to_string()),
            }
        };

        let mut res = HttpResponse::ok()
            .header("Content-Type", &stat.content_type)
            .header("ETag", &etag)
            .header("Accept-Ranges", "bytes")
            .header("Content-Length", &stat.size.to_string());

        if !is_head {
            res = res.body(data);
        }
        res
    }

    /// Handles PUT requests with If-Match optimistic concurrency (§1.3, §22.1).
    async fn handle_put(&self, req: &HttpRequest, sfs_path: &str) -> HttpResponse {
        // Transparently absorb macOS AppleDouble files and .DS_Store (§22.1)
        if is_macos_appledouble(sfs_path) {
            return HttpResponse::created();
        }

        let if_generation = req.header("if-match").and_then(|tag| {
            let clean = tag.trim_matches('"');
            if let Some(gen_str) = clean.strip_prefix("gen-") {
                gen_str.parse::<u64>().ok()
            } else {
                clean.parse::<u64>().ok()
            }
        });

        match self
            .fs
            .write_bytes(sfs_path, &req.body, if_generation)
            .await
        {
            Ok(entry) => {
                let etag = format!("\"gen-{}\"", entry.generation);
                HttpResponse::created()
                    .header("ETag", &etag)
                    .body_str("Created")
            }
            Err(SurrealFsError::Conflict(_)) => {
                HttpResponse::precondition_failed("Precondition Failed: Resource modified concurrently")
            }
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles DELETE requests.
    async fn handle_delete(&self, sfs_path: &str) -> HttpResponse {
        if is_macos_appledouble(sfs_path) {
            return HttpResponse::no_content();
        }

        match self.fs.rm(sfs_path, true).await {
            Ok(_) => HttpResponse::no_content(),
            Err(SurrealFsError::NotFound(_)) => HttpResponse::not_found("Resource not found"),
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles MKCOL requests (directory creation).
    async fn handle_mkcol(&self, req: &HttpRequest, sfs_path: &str) -> HttpResponse {
        if !req.body.is_empty() {
            return HttpResponse::new(415, "Unsupported Media Type")
                .body_str("MKCOL does not accept a request body");
        }

        match self.fs.mkdir(sfs_path, false).await {
            Ok(_) => HttpResponse::created(),
            Err(SurrealFsError::Conflict(_)) => HttpResponse::new(405, "Method Not Allowed")
                .body_str("Directory already exists"),
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles MOVE requests (renaming files or folders).
    async fn handle_move(&self, req: &HttpRequest, src_path: &str) -> HttpResponse {
        let dest_header = match req.header("destination") {
            Some(d) => d,
            None => return HttpResponse::bad_request("Missing Destination header"),
        };

        let dest_raw_path = match dest_header.split_once("://") {
            Some((_, rest)) => rest.split_once('/').map(|(_, p)| format!("/{}", p)).unwrap_or_else(|| "/".to_string()),
            None => dest_header.to_string(),
        };

        let dest_path = self.resolve_path(&dest_raw_path);
        let overwrite = req.header("overwrite").map(|v| v.to_uppercase() != "F").unwrap_or(true);

        if !overwrite && self.fs.exists(&dest_path).await.unwrap_or(false) {
            return HttpResponse::precondition_failed("Destination exists and Overwrite is F");
        }

        match self.fs.mv(src_path, &dest_path).await {
            Ok(_) => HttpResponse::created(),
            Err(SurrealFsError::NotFound(_)) => HttpResponse::not_found("Source not found"),
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles COPY requests.
    async fn handle_copy(&self, req: &HttpRequest, src_path: &str) -> HttpResponse {
        let dest_header = match req.header("destination") {
            Some(d) => d,
            None => return HttpResponse::bad_request("Missing Destination header"),
        };

        let dest_raw_path = match dest_header.split_once("://") {
            Some((_, rest)) => rest.split_once('/').map(|(_, p)| format!("/{}", p)).unwrap_or_else(|| "/".to_string()),
            None => dest_header.to_string(),
        };

        let dest_path = self.resolve_path(&dest_raw_path);
        let overwrite = req.header("overwrite").map(|v| v.to_uppercase() != "F").unwrap_or(true);

        if !overwrite && self.fs.exists(&dest_path).await.unwrap_or(false) {
            return HttpResponse::precondition_failed("Destination exists and Overwrite is F");
        }

        match self.fs.cp(src_path, &dest_path, true).await {
            Ok(_) => HttpResponse::created(),
            Err(SurrealFsError::NotFound(_)) => HttpResponse::not_found("Source not found"),
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles LOCK requests (translates to swarm advisory leases, §1.1, §22.1).
    async fn handle_lock(&self, req: &HttpRequest, sfs_path: &str) -> HttpResponse {
        let timeout_secs = req
            .header("timeout")
            .and_then(|t| {
                if let Some(s) = t.strip_prefix("Second-") {
                    s.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(60);

        match self
            .fs
            .acquire_lock(sfs_path, timeout_secs, "webdav-client", "webdav-client")
            .await
        {
            Ok(_) => {
                let token = format!("urn:uuid:{}", blake3::hash(format!("{}:{}", sfs_path, Utc::now()).as_bytes()).to_hex());
                let xml = format!(
                    r#"<?xml version="1.0" encoding="utf-8"?>
<D:prop xmlns:D="DAV:">
  <D:lockdiscovery>
    <D:activelock>
      <D:locktype><D:write/></D:locktype>
      <D:lockscope><D:exclusive/></D:lockscope>
      <D:depth>0</D:depth>
      <D:timeout>Second-{}</D:timeout>
      <D:locktoken><D:href>{}</D:href></D:locktoken>
    </D:activelock>
  </D:lockdiscovery>
</D:prop>"#,
                    timeout_secs, token
                );

                HttpResponse::ok()
                    .header("Lock-Token", &format!("<{}>", token))
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body_str(&xml)
            }
            Err(SurrealFsError::Lock(_)) => HttpResponse::locked("Resource is locked"),
            Err(e) => HttpResponse::internal_error(&e.to_string()),
        }
    }

    /// Handles UNLOCK requests (releases swarm advisory lease, §1.1).
    async fn handle_unlock(&self, sfs_path: &str) -> HttpResponse {
        match self.fs.release_lock(sfs_path, "webdav-client").await {
            Ok(_) => HttpResponse::no_content(),
            Err(_) => HttpResponse::no_content(),
        }
    }

    /// Handles PROPPATCH requests.
    fn handle_proppatch(&self, sfs_path: &str) -> HttpResponse {
        let href = self.make_href(sfs_path, false);
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>{}</D:href>
    <D:propstat>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#,
            href
        );
        HttpResponse::multi_status()
            .header("Content-Type", "application/xml; charset=utf-8")
            .body_str(&xml)
    }
}

#[async_trait]
impl HttpHandler for WebDavServer {
    async fn handle(&self, req: HttpRequest) -> HttpResponse {
        // Authenticate request if authorization header is provided
        if let Some(auth_hdr) = req.header("authorization") {
            if self.auth.authenticate_basic(auth_hdr).await.is_err() {
                return HttpResponse::unauthorized("SurrealFS WebDAV");
            }
        }

        let sfs_path = self.resolve_path(&req.path);

        match req.method.as_str() {
            "OPTIONS" => self.handle_options(),
            "PROPFIND" => self.handle_propfind(&req, &sfs_path).await,
            "GET" => self.handle_get(&req, &sfs_path, false).await,
            "HEAD" => self.handle_get(&req, &sfs_path, true).await,
            "PUT" => self.handle_put(&req, &sfs_path).await,
            "DELETE" => self.handle_delete(&sfs_path).await,
            "MKCOL" => self.handle_mkcol(&req, &sfs_path).await,
            "MOVE" => self.handle_move(&req, &sfs_path).await,
            "COPY" => self.handle_copy(&req, &sfs_path).await,
            "LOCK" => self.handle_lock(&req, &sfs_path).await,
            "UNLOCK" => self.handle_unlock(&sfs_path).await,
            "PROPPATCH" => self.handle_proppatch(&sfs_path),
            _ => HttpResponse::method_not_allowed(),
        }
    }
}

/// Identifies macOS metadata files (.DS_Store and AppleDouble ._* files).
fn is_macos_appledouble(path: &str) -> bool {
    let filename = path.split('/').next_back().unwrap_or(path);
    filename == ".DS_Store" || filename.starts_with("._")
}

/// Parses an HTTP Range header `bytes=start-end`.
fn parse_byte_range(header: &str, file_size: u64) -> Option<(u64, u64)> {
    let clean = header.trim();
    let spec = clean.strip_prefix("bytes=")?;
    let mut parts = spec.splitn(2, '-');
    let start_str = parts.next()?;
    let end_str = parts.next()?;

    if start_str.is_empty() {
        // Suffix bytes: -500
        let len: u64 = end_str.parse().ok()?;
        let start = file_size.saturating_sub(len);
        let end = file_size.saturating_sub(1);
        Some((start, end))
    } else {
        let start: u64 = start_str.parse().ok()?;
        let end = if end_str.is_empty() {
            file_size.saturating_sub(1)
        } else {
            let parsed_end: u64 = end_str.parse().ok()?;
            parsed_end.min(file_size.saturating_sub(1))
        };
        if start <= end {
            Some((start, end))
        } else {
            None
        }
    }
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
