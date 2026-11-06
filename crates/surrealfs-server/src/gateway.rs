use async_trait::async_trait;
use serde_json::json;
use surrealfs_core::SurrealFs;

use crate::api::{handle_api, handle_raw};
use crate::auth::{constant_time_eq, Authenticator};
use crate::chat::ChatConfig;
use crate::embed::{index_file_on_write, Embedder};
use crate::http::{HttpHandler, HttpRequest, HttpResponse};
use crate::mcp::handle_mcp;
use crate::s3::S3Server;
use crate::webdav::WebDavServer;

#[derive(Clone)]
pub struct SurrealFsGateway {
    pub fs: SurrealFs,
    pub embedder: Option<Embedder>,
    pub chat_cfg: Option<ChatConfig>,
    pub server_token: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    pub static_dir: Option<std::path::PathBuf>,
}

impl SurrealFsGateway {
    pub fn new(fs: SurrealFs) -> Self {
        Self {
            fs,
            embedder: None,
            chat_cfg: None,
            server_token: None,
            #[cfg(not(target_arch = "wasm32"))]
            static_dir: None,
        }
    }

    pub fn with_embedder(mut self, embedder: Option<Embedder>) -> Self {
        self.embedder = embedder;
        self
    }

    pub fn with_chat(mut self, chat_cfg: Option<ChatConfig>) -> Self {
        self.chat_cfg = chat_cfg;
        self
    }

    pub fn with_server_token(mut self, token: Option<String>) -> Self {
        self.server_token = token.filter(|t| !t.trim().is_empty());
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_static_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.static_dir = dir;
        self
    }

    async fn resolve_caller(
        &self,
        req: &HttpRequest,
    ) -> std::result::Result<SurrealFs, HttpResponse> {
        let requested_user = req
            .headers
            .get("x-surrealfs-user")
            .or_else(|| req.headers.get("x-surrealfs-caller"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(auth_hdr) = req.headers.get("authorization") {
            let trimmed = auth_hdr.trim();
            if let Some(tok) = trimmed
                .strip_prefix("Bearer ")
                .or_else(|| trimmed.strip_prefix("bearer "))
            {
                if let Some(ref expected) = self.server_token {
                    if constant_time_eq(tok.trim().as_bytes(), expected.as_bytes()) {
                        let caller = requested_user.unwrap_or_else(|| "root".to_string());
                        return Ok(self.fs.with_caller(caller));
                    }
                }
            }

            let auth = Authenticator::new(self.fs.clone());
            if let Ok(user) = auth.authenticate_s3(Some(trimmed)).await {
                return Ok(self.fs.with_caller(user.user_id));
            }

            if self.server_token.is_some() {
                return Err(HttpResponse::json(
                    401,
                    &json!({ "error": "Invalid authentication credentials" }),
                ));
            }
        } else if self.server_token.is_some() {
            return Err(HttpResponse::json(
                401,
                &json!({ "error": "Authentication required" }),
            ));
        }

        // Anonymous request on a server with no token: the identity headers are
        // unauthenticated and must not select the caller.
        Ok(self.fs.clone())
    }

    fn is_s3_request(&self, req: &HttpRequest) -> bool {
        let p = req.clean_path();
        if p == "/s3" || p.starts_with("/s3/") {
            return true;
        }
        if req.headers.contains_key("x-amz-content-sha256")
            || req.headers.contains_key("x-amz-date")
        {
            return true;
        }
        if let Some(auth) = req.headers.get("authorization") {
            if auth.starts_with("AWS4-HMAC-SHA256") {
                return true;
            }
        }
        false
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl HttpHandler for SurrealFsGateway {
    async fn handle(&self, req: HttpRequest) -> HttpResponse {
        if req.method == "OPTIONS" {
            return HttpResponse::empty(204);
        }

        let clean = req.clean_path().to_string();

        // Public health check
        if clean == "/api/health" || clean == "/health" {
            return handle_api(
                &self.fs,
                self.embedder.as_ref(),
                self.chat_cfg.as_ref(),
                &req,
            )
            .await;
        }

        // Static UI assets on GET when not hitting a protocol endpoint
        #[cfg(not(target_arch = "wasm32"))]
        if req.method == "GET"
            && !clean.starts_with("/api/")
            && clean != "/raw"
            && !clean.starts_with("/mcp")
            && !clean.starts_with("/dav")
            && !self.is_s3_request(&req)
        {
            if let Some(ref dir) = self.static_dir {
                if let Some(resp) = serve_static_file(dir, &clean) {
                    return resp;
                }
            }
        }

        let scoped_fs = match self.resolve_caller(&req).await {
            Ok(fs) => fs,
            Err(resp) => return resp,
        };

        if clean.starts_with("/api/") {
            return handle_api(
                &scoped_fs,
                self.embedder.as_ref(),
                self.chat_cfg.as_ref(),
                &req,
            )
            .await;
        }

        if clean == "/raw" {
            return handle_raw(&scoped_fs, &req).await;
        }

        if clean == "/mcp" || clean.starts_with("/mcp/") {
            return handle_mcp(&scoped_fs, self.embedder.as_ref(), &req).await;
        }

        if clean == "/dav" || clean.starts_with("/dav/") {
            let is_put = req.method == "PUT";
            let stripped = clean.strip_prefix("/dav").unwrap_or("/");
            let target_path = if stripped.is_empty() {
                "/".to_string()
            } else {
                stripped.to_string()
            };
            let dav = WebDavServer::new(scoped_fs.clone(), "/dav");
            let resp = dav.handle(req).await;
            if is_put && (200..300).contains(&resp.status) {
                let _ = index_file_on_write(&scoped_fs, self.embedder.as_ref(), &target_path).await;
            }
            return resp;
        }

        if self.is_s3_request(&req) {
            let mut s3_req = req.clone();
            if clean == "/s3" || clean.starts_with("/s3/") {
                let stripped = req.path.strip_prefix("/s3").unwrap_or("/");
                s3_req.path = if stripped.is_empty() {
                    "/".to_string()
                } else {
                    stripped.to_string()
                };
            }
            let is_put = s3_req.method == "PUT";
            let s3_clean = s3_req.clean_path().to_string();
            let s3 = S3Server::new(scoped_fs.clone());
            let resp = s3.handle(s3_req).await;
            if is_put && (200..300).contains(&resp.status) {
                let trimmed = s3_clean.trim_start_matches('/');
                if let Some((_bucket, key)) = trimmed.split_once('/') {
                    let vfs_path = format!("/{}", key);
                    let _ =
                        index_file_on_write(&scoped_fs, self.embedder.as_ref(), &vfs_path).await;
                }
            }
            return resp;
        }

        HttpResponse::json(
            200,
            &json!({
                "service": "surrealfs-server",
                "version": env!("CARGO_PKG_VERSION"),
                "surfaces": {
                    "api": "/api/*",
                    "rpc": "/api/rpc",
                    "mcp": "/mcp",
                    "webdav": "/dav/*",
                    "s3": "/s3/*",
                    "chat": "/api/chat"
                }
            }),
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn serve_static_file(static_dir: &std::path::Path, clean_path: &str) -> Option<HttpResponse> {
    if clean_path.split('/').any(|seg| seg == "..") {
        return Some(HttpResponse::json(
            403,
            &json!({ "error": "Path traversal forbidden" }),
        ));
    }

    let rel = clean_path.trim_start_matches('/');
    let candidate = if rel.is_empty() {
        static_dir.join("index.html")
    } else {
        static_dir.join(rel)
    };

    let target = if candidate.is_file() {
        if let (Ok(canon_dir), Ok(canon_file)) =
            (static_dir.canonicalize(), candidate.canonicalize())
        {
            if !canon_file.starts_with(&canon_dir) {
                return Some(HttpResponse::json(
                    403,
                    &json!({ "error": "Path traversal forbidden" }),
                ));
            }
        }
        candidate
    } else {
        let has_asset_ext = candidate
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                matches!(
                    ext.to_ascii_lowercase().as_str(),
                    "js" | "mjs"
                        | "css"
                        | "svg"
                        | "png"
                        | "jpg"
                        | "jpeg"
                        | "gif"
                        | "webp"
                        | "woff"
                        | "woff2"
                        | "ttf"
                        | "ico"
                        | "map"
                )
            });
        if clean_path.starts_with("/assets/") || has_asset_ext {
            return Some(HttpResponse::json(
                404,
                &json!({ "error": format!("Static asset not found: {}", clean_path) }),
            ));
        }
        let index = static_dir.join("index.html");
        if index.is_file() {
            index
        } else {
            return None;
        }
    };

    let bytes = std::fs::read(&target).ok()?;
    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let ct = match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "json" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    };
    Some(HttpResponse::with_bytes(200, bytes, ct))
}

/// Rejects serving an unauthenticated gateway on a non-loopback address.
///
/// Without a server token every request runs as the gateway's own identity,
/// so only a loopback bind is safe unless the operator opts in with
/// `SURREALFS_ALLOW_ANONYMOUS=1`.
pub fn check_exposure(addr: &str, server_token: Option<&str>) -> std::result::Result<(), String> {
    if server_token.is_some_and(|t| !t.trim().is_empty()) {
        return Ok(());
    }
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_matches(|c| c == '[' || c == ']');
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    let allowed = std::env::var("SURREALFS_ALLOW_ANONYMOUS")
        .is_ok_and(|v| matches!(v.trim(), "1" | "true" | "yes"));
    if loopback || allowed {
        Ok(())
    } else {
        Err(format!(
            "refusing to serve {addr} without authentication: set SURREALFS_SERVER_TOKEN, \
             bind a loopback address, or set SURREALFS_ALLOW_ANONYMOUS=1"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::check_exposure;

    #[test]
    fn exposure_guard_requires_token_off_loopback() {
        assert!(check_exposure("127.0.0.1:8787", None).is_ok());
        assert!(check_exposure("[::1]:8787", None).is_ok());
        assert!(check_exposure("localhost:8787", None).is_ok());
        assert!(check_exposure("0.0.0.0:8787", Some("secret")).is_ok());
        if std::env::var_os("SURREALFS_ALLOW_ANONYMOUS").is_none() {
            assert!(check_exposure("0.0.0.0:8787", None).is_err());
            assert!(check_exposure("0.0.0.0:8787", Some("  ")).is_err());
            assert!(check_exposure("10.0.0.5:8787", None).is_err());
        }
    }
}
