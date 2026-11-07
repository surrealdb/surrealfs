//! Lightweight, robust asynchronous HTTP/1.1 protocol engine for WebDAV and S3 (§22).

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
#[cfg(not(target_arch = "wasm32"))]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(not(target_arch = "wasm32"))]
use tokio::net::TcpListener;
#[cfg(not(target_arch = "wasm32"))]
use tracing::debug;

/// A body that is read once, as it arrives or as it is sent, so that it never has to be held
/// whole in memory.
///
/// Clones share the stream; the first [`take`](Self::take) gets it.
#[derive(Clone)]
pub struct BodyStream(Arc<Mutex<Option<surrealfs_core::ByteStream>>>);

impl BodyStream {
    pub fn new(stream: surrealfs_core::ByteStream) -> Self {
        Self(Arc::new(Mutex::new(Some(stream))))
    }

    pub fn take(&self) -> Option<surrealfs_core::ByteStream> {
        self.0.lock().unwrap().take()
    }
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BodyStream")
    }
}

/// Incoming HTTP request (§22).
///
/// A request body larger than [`STREAM_BODY_THRESHOLD`] is not held in `body`: `body_stream`
/// carries it, and `body` is empty.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub body_stream: Option<BodyStream>,
    pub method: String,
    pub path: String,
    pub raw_query: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Construct an `HttpRequest` from method, target (path + optional `?query`), headers, and body.
    pub fn from_parts(
        method: &str,
        full_target: &str,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    ) -> Self {
        let (raw_path, raw_query) = match full_target.split_once('?') {
            Some((p, q)) => (p, q),
            None => (full_target, ""),
        };
        let path = urlencoding_decode(raw_path);
        let mut query = HashMap::new();
        if !raw_query.is_empty() {
            for pair in raw_query.split('&') {
                let mut kv = pair.splitn(2, '=');
                let k = urlencoding_decode(kv.next().unwrap_or(""));
                let v = urlencoding_decode(kv.next().unwrap_or(""));
                query.insert(k, v);
            }
        }
        let mut lower_headers = HashMap::with_capacity(headers.len());
        for (k, v) in headers {
            lower_headers.insert(k.to_ascii_lowercase(), v);
        }
        Self {
            method: method.to_uppercase(),
            path,
            raw_query: raw_query.to_string(),
            query,
            headers: lower_headers,
            body,
            body_stream: None,
        }
    }

    /// The body as a stream: the streamed body, or `body` as one piece.
    pub fn into_body_stream(&self) -> surrealfs_core::ByteStream {
        if let Some(stream) = self.body_stream.as_ref().and_then(|b| b.take()) {
            return stream;
        }
        let body = self.body.clone();
        Box::pin(futures::stream::once(async move { Ok(body) }))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(|s| s.as_str())
    }

    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(|s| s.as_str())
    }

    pub fn clean_path(&self) -> &str {
        &self.path
    }

    pub fn query_params(&self) -> &HashMap<String, String> {
        &self.query
    }
}

/// Outgoing HTTP response (§22).
///
/// With `stream` set, the body is the stream and `body` is empty; the `content-length` header
/// names its length.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub stream: Option<BodyStream>,
}

impl HttpResponse {
    pub fn new(status: u16, status_text: &str) -> Self {
        Self {
            status,
            status_text: status_text.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
            stream: None,
        }
    }

    /// Send `stream` as the body, `len` bytes long.
    pub fn streaming(mut self, stream: surrealfs_core::ByteStream, len: u64) -> Self {
        self.stream = Some(BodyStream::new(stream));
        self.headers
            .insert("content-length".to_string(), len.to_string());
        self
    }

    /// The whole body: `body`, or the streamed body read to its end.
    pub async fn into_bytes(self) -> Vec<u8> {
        use futures::StreamExt;
        let Some(mut stream) = self.stream.as_ref().and_then(|s| s.take()) else {
            return self.body;
        };
        let mut out = Vec::new();
        while let Some(piece) = stream.next().await {
            match piece {
                Ok(p) => out.extend_from_slice(&p),
                Err(_) => break,
            }
        }
        out
    }

    pub fn empty(status: u16) -> Self {
        let status_text = match status {
            200 => "OK",
            202 => "Accepted",
            204 => "No Content",
            _ => "OK",
        };
        Self::new(status, status_text)
    }

    pub fn with_bytes(status: u16, body: Vec<u8>, content_type: &str) -> Self {
        let status_text = match status {
            200 => "OK",
            201 => "Created",
            _ => "OK",
        };
        Self::new(status, status_text)
            .header("Content-Type", content_type)
            .body(body)
    }

    pub fn json<T: serde::Serialize>(status: u16, payload: &T) -> Self {
        let status_text = match status {
            200 => "OK",
            201 => "Created",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            409 => "Conflict",
            412 => "Precondition Failed",
            423 => "Locked",
            _ => "Error",
        };
        let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
        Self::new(status, status_text)
            .header("Content-Type", "application/json; charset=utf-8")
            .body(body)
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_lowercase(), value.to_string());
        self
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    pub fn body(mut self, bytes: Vec<u8>) -> Self {
        self.body = bytes;
        self
    }

    pub fn body_str(mut self, text: &str) -> Self {
        self.body = text.as_bytes().to_vec();
        self
    }

    pub fn ok() -> Self {
        Self::new(200, "OK")
    }

    pub fn created() -> Self {
        Self::new(201, "Created")
    }

    pub fn no_content() -> Self {
        Self::new(204, "No Content")
    }

    pub fn partial_content() -> Self {
        Self::new(206, "Partial Content")
    }

    pub fn multi_status() -> Self {
        Self::new(207, "Multi-Status")
    }

    pub fn not_modified() -> Self {
        Self::new(304, "Not Modified")
    }

    pub fn bad_request(msg: &str) -> Self {
        Self::new(400, "Bad Request")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn unauthorized(realm: &str) -> Self {
        Self::new(401, "Unauthorized")
            .header("WWW-Authenticate", &format!("Basic realm=\"{}\"", realm))
            .header("Content-Type", "text/plain")
            .body_str("Unauthorized")
    }

    pub fn forbidden(msg: &str) -> Self {
        Self::new(403, "Forbidden")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn not_found(msg: &str) -> Self {
        Self::new(404, "Not Found")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn method_not_allowed() -> Self {
        Self::new(405, "Method Not Allowed")
    }

    pub fn conflict(msg: &str) -> Self {
        Self::new(409, "Conflict")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn precondition_failed(msg: &str) -> Self {
        Self::new(412, "Precondition Failed")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn locked(msg: &str) -> Self {
        Self::new(423, "Locked")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }

    pub fn internal_error(msg: &str) -> Self {
        Self::new(500, "Internal Server Error")
            .header("Content-Type", "text/plain; charset=utf-8")
            .body_str(msg)
    }
}

/// Handler for processing HTTP requests asynchronously (§22).
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait HttpHandler: Send + Sync {
    async fn handle(&self, req: HttpRequest) -> HttpResponse;
}

/// Maximum allowed size of an HTTP request body that is held in memory (100 MiB).
pub const MAX_HTTP_BODY_BYTES: usize = 100 * 1024 * 1024;

/// A `PUT` body longer than this is streamed to its handler instead of being read first.
pub const STREAM_BODY_THRESHOLD: usize = surrealfs_core::INLINE_MAX_BYTES;

/// Maximum size of a streamed `PUT` body (5 TiB, the largest S3 object).
pub const MAX_STREAMED_BODY_BYTES: u64 = 5 * 1024 * 1024 * 1024 * 1024;

/// Parses an incoming raw HTTP request from a TCP stream.
#[cfg(not(target_arch = "wasm32"))]
pub async fn parse_request<R>(mut stream: R) -> std::io::Result<Option<HttpRequest>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = Vec::new();
    let mut temp = [0u8; 4096];
    let mut header_end = None;
    let read_timeout = std::time::Duration::from_secs(30);

    // Read headers until \r\n\r\n or \n\n
    while header_end.is_none() {
        let n = tokio::time::timeout(read_timeout, stream.read(&mut temp))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "HTTP header read timed out")
            })??;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        buf.extend_from_slice(&temp[..n]);

        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = Some((pos, pos + 4));
        } else if let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
            header_end = Some((pos, pos + 2));
        }

        if buf.len() > 65536 && header_end.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP headers too large",
            ));
        }
    }

    let (headers_len, body_start) = match header_end {
        Some(pair) => pair,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Incomplete HTTP request headers",
            ))
        }
    };

    let header_str = String::from_utf8_lossy(&buf[..headers_len]);
    let mut lines = header_str.lines();

    let request_line = match lines.next() {
        Some(l) if !l.trim().is_empty() => l.trim(),
        _ => return Ok(None),
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_uppercase();
    let full_target = parts.next().unwrap_or("/");

    let (raw_path, raw_query) = match full_target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (full_target, ""),
    };

    let path = urlencoding_decode(raw_path);

    let mut query = HashMap::new();
    if !raw_query.is_empty() {
        for pair in raw_query.split('&') {
            let mut kv = pair.splitn(2, '=');
            let k = urlencoding_decode(kv.next().unwrap_or(""));
            let v = urlencoding_decode(kv.next().unwrap_or(""));
            query.insert(k, v);
        }
    }

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    // Read remaining body based on Content-Length
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let streamed = method == "PUT" && content_length > STREAM_BODY_THRESHOLD;
    if streamed && content_length as u64 > MAX_STREAMED_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP request body exceeds maximum size limit",
        ));
    }
    if !streamed && content_length > MAX_HTTP_BODY_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "HTTP request body exceeds maximum size limit",
        ));
    }

    let mut body = buf[body_start..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }

    if streamed {
        let remaining = content_length - body.len();
        let first = std::mem::take(&mut body);
        let body_stream = stream_from_reader(first, stream, remaining, read_timeout);
        return Ok(Some(HttpRequest {
            method,
            path,
            raw_query: raw_query.to_string(),
            query,
            headers,
            body,
            body_stream: Some(BodyStream::new(body_stream)),
        }));
    }

    while body.len() < content_length {
        let needed = content_length - body.len();
        let to_read = needed.min(temp.len());
        let n = tokio::time::timeout(read_timeout, stream.read(&mut temp[..to_read]))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "HTTP body read timed out")
            })??;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&temp[..n]);
    }

    Ok(Some(HttpRequest {
        method,
        path,
        raw_query: raw_query.to_string(),
        query,
        headers,
        body,
        body_stream: None,
    }))
}

/// The remaining `remaining` bytes of a request body, after the `first` bytes already read,
/// as a stream of pieces of at most 256 KiB. A connection that ends early ends the stream with
/// an error, so a truncated upload is never committed.
#[cfg(not(target_arch = "wasm32"))]
fn stream_from_reader<R>(
    first: Vec<u8>,
    reader: R,
    remaining: usize,
    timeout: std::time::Duration,
) -> surrealfs_core::ByteStream
where
    R: AsyncRead + Unpin + Send + 'static,
{
    use surrealfs_core::SurrealFsError;
    let head = futures::stream::iter(if first.is_empty() {
        None
    } else {
        Some(Ok(first))
    });
    let tail =
        futures::stream::try_unfold((reader, remaining), move |(mut reader, left)| async move {
            if left == 0 {
                return Ok(None);
            }
            let mut piece = vec![0u8; left.min(256 * 1024)];
            let n = tokio::time::timeout(timeout, reader.read(&mut piece))
                .await
                .map_err(|_| SurrealFsError::Io("HTTP body read timed out".to_string()))?
                .map_err(|e| SurrealFsError::Io(e.to_string()))?;
            if n == 0 {
                return Err(SurrealFsError::Io(
                    "connection closed before the whole body arrived".to_string(),
                ));
            }
            piece.truncate(n);
            Ok(Some((piece, (reader, left - n))))
        });
    Box::pin(futures::StreamExt::chain(head, tail))
}

/// Serializes and sends an HTTP response to a TCP stream.
#[cfg(not(target_arch = "wasm32"))]
pub async fn send_response<W>(stream: &mut W, res: HttpResponse) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut head = format!("HTTP/1.1 {} {}\r\n", res.status, res.status_text);

    let mut headers = res.headers;
    if !headers.contains_key("content-length") {
        headers.insert("content-length".to_string(), res.body.len().to_string());
    }
    if !headers.contains_key("connection") {
        headers.insert("connection".to_string(), "close".to_string());
    }

    for (k, v) in headers {
        head.push_str(&format!("{}: {}\r\n", k, v));
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes()).await?;
    if let Some(body) = res.stream.as_ref().and_then(|s| s.take()) {
        use futures::StreamExt;
        let mut body = body;
        while let Some(piece) = body.next().await {
            let piece = piece.map_err(|e| std::io::Error::other(e.to_string()))?;
            stream.write_all(&piece).await?;
        }
    } else if !res.body.is_empty() {
        stream.write_all(&res.body).await?;
    }
    stream.flush().await?;
    Ok(())
}

/// Minimal percent-decoding helper.
pub fn urlencoding_decode(input: &str) -> String {
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Serves an HTTP handler on a given socket address.
#[cfg(not(target_arch = "wasm32"))]
pub async fn run_http_server<H: HttpHandler + 'static>(
    addr: &str,
    handler: Arc<H>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("SurrealFS HTTP server listening on {}", addr);

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let handler = Arc::clone(&handler);

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            match parse_request(reader).await {
                Ok(Some(req)) => {
                    let res = handler.handle(req).await;
                    if let Err(e) = send_response(&mut writer, res).await {
                        debug!("Failed to send HTTP response to {}: {}", peer_addr, e);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    debug!("Failed to parse HTTP request from {}: {}", peer_addr, e);
                    let resp = if e.to_string().contains("exceeds maximum size limit") {
                        HttpResponse::new(413, "Payload Too Large")
                            .header("Content-Type", "text/plain; charset=utf-8")
                            .body_str("Payload Too Large")
                    } else {
                        HttpResponse::bad_request("Malformed request")
                    };
                    let _ = send_response(&mut writer, resp).await;
                }
            }
        });
    }
}

/// Convenience helper to serve an owned `HttpHandler` on a TCP socket address.
#[cfg(not(target_arch = "wasm32"))]
pub async fn serve_http<H: HttpHandler + 'static>(addr: &str, handler: H) -> std::io::Result<()> {
    run_http_server(addr, Arc::new(handler)).await
}
