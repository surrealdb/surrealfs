//! Lightweight, robust asynchronous HTTP/1.1 protocol engine for WebDAV and S3 (§22).

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::debug;

/// Incoming HTTP request (§22).
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub raw_query: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(|s| s.as_str())
    }

    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(|s| s.as_str())
    }
}

/// Outgoing HTTP response (§22).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16, status_text: &str) -> Self {
        Self {
            status,
            status_text: status_text.to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
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
#[async_trait]
pub trait HttpHandler: Send + Sync {
    async fn handle(&self, req: HttpRequest) -> HttpResponse;
}

/// Parses an incoming raw HTTP request from a TCP stream.
pub async fn parse_request(stream: &mut TcpStream) -> std::io::Result<Option<HttpRequest>> {
    let mut buf = Vec::new();
    let mut temp = [0u8; 4096];
    let mut header_end = None;

    // Read headers until \r\n\r\n or \n\n
    while header_end.is_none() {
        let n = stream.read(&mut temp).await?;
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

    let mut body = buf[body_start..].to_vec();
    while body.len() < content_length {
        let needed = content_length - body.len();
        let to_read = needed.min(temp.len());
        let n = stream.read(&mut temp[..to_read]).await?;
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
    }))
}

/// Serializes and sends an HTTP response to a TCP stream.
pub async fn send_response(stream: &mut TcpStream, res: HttpResponse) -> std::io::Result<()> {
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
    if !res.body.is_empty() {
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
            if let Ok(byte) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
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
pub async fn run_http_server<H: HttpHandler + 'static>(
    addr: &str,
    handler: Arc<H>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("SurrealFS HTTP server listening on {}", addr);

    loop {
        let (mut stream, peer_addr) = listener.accept().await?;
        let handler = Arc::clone(&handler);

        tokio::spawn(async move {
            match parse_request(&mut stream).await {
                Ok(Some(req)) => {
                    let res = handler.handle(req).await;
                    if let Err(e) = send_response(&mut stream, res).await {
                        debug!("Failed to send HTTP response to {}: {}", peer_addr, e);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    debug!("Failed to parse HTTP request from {}: {}", peer_addr, e);
                    let _ = send_response(&mut stream, HttpResponse::bad_request("Malformed request")).await;
                }
            }
        });
    }
}
