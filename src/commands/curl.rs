use reqwest::StatusCode;
use surrealdb::Connection;

use surrealfs::curl::{self, CurlOutput, CurlRequest, CurlResult};
use surrealfs::{FsError, SurrealFs};

use super::ReplState;
use super::util::{help_error, resolve_cli_path};

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
    pub status: StatusCode,
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
    let resp = run_curl(&state.fs, opts, OutputMode::Capture).await?;
    Ok(CurlResponse {
        status: resp.status,
        body: resp.body,
    })
}

fn parse_curl_args(args: &[&str], cwd: &str) -> Result<CurlRequest, FsError> {
    let mut follow = false;
    let mut headers = Vec::new();
    let mut data = None;
    let mut method = None;
    let mut output = None;
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
                output = Some(CurlOutput::Path(resolve_cli_path(cwd, args[i + 1])));
                i += 2;
            }
            "-O" => {
                output = Some(CurlOutput::AutoName);
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
    Ok(CurlRequest {
        url,
        follow,
        headers,
        data,
        method,
        output,
    })
}

enum OutputMode {
    Print,
    Capture,
}

async fn run_curl<DB>(
    fs: &SurrealFs<DB>,
    request: CurlRequest,
    mode: OutputMode,
) -> Result<CurlResult, FsError>
where
    DB: Connection,
{
    let resp = curl::curl(fs, request).await?;

    if let OutputMode::Print = mode {
        if let Some(saved) = &resp.saved_to {
            println!("Saved to {} (status {})", saved, resp.status);
        } else {
            println!("Status: {}", resp.status);
            print!("{}", resp.body);
        }
    }

    Ok(resp)
}
