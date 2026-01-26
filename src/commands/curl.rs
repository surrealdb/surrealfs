use reqwest::{Client, Url};
use surrealdb::Connection;

use surrealfs::{FsError, SurrealFs};

use super::ReplState;
use super::util::{help_error, resolve_cli_path};

#[derive(Debug)]
struct CurlOptions {
    url: String,
    follow: bool,
    headers: Vec<(String, String)>,
    data: Option<String>,
    method: Option<String>,
    out: Option<String>,
}

pub async fn run<DB>(args: &[&str], state: &mut ReplState<DB>) -> Result<(), FsError>
where
    DB: Connection,
{
    let opts = parse_curl_args(args, &state.cwd)?;
    run_curl(&state.fs, opts, OutputMode::Print)
        .await
        .map(|_| ())
}

#[derive(Debug, Clone)]
pub struct CurlResponse {
    pub status: reqwest::StatusCode,
    pub body: String,
}

pub async fn run_capture<DB>(
    args: &[&str],
    state: &mut ReplState<DB>,
) -> Result<CurlResponse, FsError>
where
    DB: Connection,
{
    let opts = parse_curl_args(args, &state.cwd)?;
    run_curl(&state.fs, opts, OutputMode::Capture)
        .await
        .map(|resp| CurlResponse {
            status: resp.status,
            body: resp.body,
        })
}

fn parse_curl_args(args: &[&str], cwd: &str) -> Result<CurlOptions, FsError> {
    let mut follow = false;
    let mut headers = Vec::new();
    let mut data = None;
    let mut method = None;
    let mut out = None;
    let mut url = None;

    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "-L" => {
                follow = true;
                i += 1;
            }
            "-H" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                let h = args[i + 1];
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
                i += 2;
            }
            "-d" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                data = Some(args[i + 1].to_string());
                i += 2;
            }
            "-X" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                method = Some(args[i + 1].to_string());
                i += 2;
            }
            "-o" => {
                if i + 1 >= args.len() {
                    return Err(help_error());
                }
                out = Some(resolve_cli_path(cwd, args[i + 1]));
                i += 2;
            }
            "-O" => {
                out = Some(String::new());
                i += 1;
            }
            other => {
                if other.starts_with('-') {
                    return Err(help_error());
                }
                url = Some(other.to_string());
                i += 1;
            }
        }
    }

    let url = url.ok_or_else(help_error)?;
    Ok(CurlOptions {
        url,
        follow,
        headers,
        data,
        method,
        out,
    })
}

enum OutputMode {
    Print,
    Capture,
}

async fn run_curl<DB>(
    fs: &SurrealFs<DB>,
    opts: CurlOptions,
    mode: OutputMode,
) -> Result<CurlResponse, FsError>
where
    DB: Connection,
{
    let mut client = Client::builder();
    if opts.follow {
        client = client.redirect(reqwest::redirect::Policy::limited(10));
    } else {
        client = client.redirect(reqwest::redirect::Policy::none());
    }
    let client = client.build().map_err(|e| FsError::Http(e.to_string()))?;

    let method = opts
        .method
        .clone()
        .unwrap_or_else(|| if opts.data.is_some() { "POST" } else { "GET" }.to_string());

    let mut req = client.request(method.parse().unwrap_or(reqwest::Method::GET), &opts.url);

    for (k, v) in &opts.headers {
        req = req.header(k, v);
    }

    if let Some(body) = &opts.data {
        req = req.body(body.clone());
    }

    let resp = req.send().await.map_err(|e| FsError::Http(e.to_string()))?;
    let status = resp.status();
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| FsError::Http(e.to_string()))?;

    let body = String::from_utf8_lossy(&bytes).to_string();

    match mode {
        OutputMode::Print => {
            if let Some(out_path) = &opts.out {
                let target = if out_path.is_empty() {
                    derive_out_name(&opts.url)
                } else {
                    out_path.clone()
                };
                fs.write_file(&target, body.clone()).await?;
                println!("Saved to {} (status {})", target, status);
            } else {
                println!("Status: {}", status);
                print!("{}", body);
            }
        }
        OutputMode::Capture => {}
    }

    if !status.is_success() {
        return Err(FsError::Http(format!("HTTP status {}", status)));
    }

    Ok(CurlResponse { status, body })
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
