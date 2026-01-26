use reqwest::{Client, Method, StatusCode, Url};
use surrealdb::Connection;

use crate::{FsError, SurrealFs};

#[derive(Debug, Clone)]
pub struct CurlRequest {
    pub url: String,
    pub follow: bool,
    pub headers: Vec<(String, String)>,
    pub data: Option<String>,
    pub method: Option<String>,
    pub output: Option<CurlOutput>,
}

#[derive(Debug, Clone)]
pub enum CurlOutput {
    Path(String),
    AutoName,
}

#[derive(Debug, Clone)]
pub struct CurlResult {
    pub status: StatusCode,
    pub body: String,
    pub saved_to: Option<String>,
}

pub async fn curl<DB>(fs: &SurrealFs<DB>, request: CurlRequest) -> Result<CurlResult, FsError>
where
    DB: Connection,
{
    let mut client = Client::builder();
    if request.follow {
        client = client.redirect(reqwest::redirect::Policy::limited(10));
    } else {
        client = client.redirect(reqwest::redirect::Policy::none());
    }
    let client = client.build().map_err(|e| FsError::Http(e.to_string()))?;

    let method = request
        .method
        .as_deref()
        .unwrap_or(if request.data.is_some() {
            "POST"
        } else {
            "GET"
        });

    let mut req = client.request(
        method.parse::<Method>().unwrap_or(Method::GET),
        &request.url,
    );

    for (k, v) in &request.headers {
        req = req.header(k, v);
    }

    if let Some(body) = &request.data {
        req = req.body(body.clone());
    }

    let resp = req.send().await.map_err(|e| FsError::Http(e.to_string()))?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| FsError::Http(e.to_string()))?;

    let body = String::from_utf8_lossy(&bytes).to_string();

    let saved_to = if let Some(output) = request.output {
        let target = match output {
            CurlOutput::Path(path) => path,
            CurlOutput::AutoName => derive_out_name(&request.url),
        };
        fs.write_file(&target, body.clone()).await?;
        Some(target)
    } else {
        None
    };

    if !status.is_success() {
        return Err(FsError::Http(format!("HTTP status {}", status)));
    }

    Ok(CurlResult {
        status,
        body,
        saved_to,
    })
}

fn derive_out_name(url: &str) -> String {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(seg) = parsed
            .path_segments()
            .and_then(|s| s.filter(|v| !v.is_empty()).last())
        {
            return seg.to_string();
        }
    }
    "index.html".to_string()
}
