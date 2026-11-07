//! Serving file content over HTTP: `Range`, validators and a streamed body, shared by the S3,
//! WebDAV and raw routes.

use surrealfs_core::{ByteRange, SurrealFs, SurrealFsError};

use crate::http::{HttpRequest, HttpResponse};

/// The validator of a file at `generation`.
pub fn etag(generation: u64) -> String {
    format!("\"gen-{generation}\"")
}

/// The single byte range of a `Range` header, or `None` when there is none or it is not a
/// single `bytes=` range (a multi-range request is answered with the whole file, as RFC 9110
/// allows).
pub fn parse_range_header(header: &str) -> Option<ByteRange> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = (start.trim(), end.trim());
    if start.is_empty() {
        return Some(ByteRange::Suffix(end.parse().ok()?));
    }
    let start: u64 = start.parse().ok()?;
    if end.is_empty() {
        return Some(ByteRange::From(start));
    }
    Some(ByteRange::Bounded(start, end.parse().ok()?))
}

/// Why a file could not be served.
pub enum ServeError {
    /// The requested range selects no byte; answer `416` with the size.
    NotSatisfiable(u64),
    Fs(SurrealFsError),
}

impl From<SurrealFsError> for ServeError {
    fn from(e: SurrealFsError) -> Self {
        ServeError::Fs(e)
    }
}

/// Open `path` and answer `req` (a `GET` or `HEAD`) with its content.
///
/// The answer is `200`, `206` for a `Range`, or `304` when `If-None-Match` names the file's
/// current validator. A `HEAD` carries the headers a `GET` would and no body. `content_type`
/// overrides the stored type.
pub async fn serve_file(
    fs: &SurrealFs,
    req: &HttpRequest,
    path: &str,
    content_type: Option<&str>,
) -> Result<HttpResponse, ServeError> {
    let range = req
        .header("range")
        .and_then(parse_range_header)
        .unwrap_or(ByteRange::Full);
    let file = match fs.read_stream(path, range).await {
        Ok(f) => f,
        Err(SurrealFsError::InvalidArguments(_)) => {
            let size = fs.stat(path).await?.map(|e| e.size).unwrap_or_default();
            if size > 0 {
                return Err(ServeError::NotSatisfiable(size));
            }
            // No range selects a byte of an empty file; it is answered as a whole read.
            fs.read_stream(path, ByteRange::Full).await?
        }
        Err(e) => return Err(e.into()),
    };

    let tag = etag(file.generation);
    if let Some(candidate) = req.header("if-none-match") {
        if candidate == "*" || candidate == tag {
            return Ok(HttpResponse::not_modified().header("ETag", &tag));
        }
    }

    let len = file.range.end - file.range.start;
    let partial = file.range.start != 0 || file.range.end != file.size;
    let mut res = if partial {
        HttpResponse::partial_content().header(
            "Content-Range",
            &format!(
                "bytes {}-{}/{}",
                file.range.start,
                file.range.end - 1,
                file.size
            ),
        )
    } else {
        HttpResponse::ok()
    }
    .header(
        "Content-Type",
        content_type.unwrap_or(file.content_type.as_str()),
    )
    .header("ETag", &tag)
    .header("Accept-Ranges", "bytes");

    if req.method == "HEAD" {
        res.headers
            .insert("content-length".to_string(), len.to_string());
        return Ok(res);
    }
    Ok(res.streaming(file.body, len))
}

/// The `416` answer for a file of `size` bytes.
pub fn range_not_satisfiable(size: u64) -> HttpResponse {
    HttpResponse::new(416, "Range Not Satisfiable")
        .header("Content-Range", &format!("bytes */{size}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_headers() {
        assert_eq!(
            parse_range_header("bytes=0-99"),
            Some(ByteRange::Bounded(0, 99))
        );
        assert_eq!(parse_range_header("bytes=5-"), Some(ByteRange::From(5)));
        assert_eq!(parse_range_header("bytes=-7"), Some(ByteRange::Suffix(7)));
        assert_eq!(parse_range_header("bytes=0-1,5-9"), None);
        assert_eq!(parse_range_header("items=0-1"), None);
        assert_eq!(parse_range_header("bytes=a-b"), None);
    }
}
