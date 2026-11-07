use js_sys::{Object, Reflect, Uint8Array};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use surrealfs_core::{ByteStream, ConnectOptions, SurrealFs};
use wasm_bindgen::prelude::*;

use crate::chat::ChatConfig;
use crate::embed::Embedder;
use crate::gateway::SurrealFsGateway;
use crate::http::{BodyStream, HttpHandler, HttpRequest};
use crate::r2::{call, R2BlobStore};

#[derive(Deserialize, Default)]
pub struct WorkerEnv {
    #[serde(rename = "SURREALDB_URL", default)]
    pub surrealdb_url: String,
    #[serde(rename = "SURREALDB_NS", default)]
    pub surrealdb_ns: Option<String>,
    #[serde(rename = "SURREALDB_DB", default)]
    pub surrealdb_db: Option<String>,
    #[serde(rename = "SURREALDB_USER", default)]
    pub surrealdb_user: Option<String>,
    #[serde(rename = "SURREALDB_PASS", default)]
    pub surrealdb_pass: Option<String>,
    #[serde(rename = "SURREALDB_TOKEN", default)]
    pub surrealdb_token: Option<String>,
    #[serde(rename = "OPENAI_API_KEY", default)]
    pub openai_api_key: Option<String>,
    #[serde(rename = "OPENAI_BASE_URL", default)]
    pub openai_base_url: Option<String>,
    #[serde(rename = "SURREALFS_EMBED_MODEL", default)]
    pub embed_model: Option<String>,
    #[serde(rename = "ANTHROPIC_API_KEY", default)]
    pub anthropic_api_key: Option<String>,
    #[serde(rename = "SURREALFS_CHAT_MODEL", default)]
    pub chat_model: Option<String>,
    #[serde(rename = "SURREALFS_SERVER_TOKEN", default)]
    pub server_token: Option<String>,
}

/// The body of a response that is too large to hold: the Worker pulls it a piece at a time.
#[wasm_bindgen]
pub struct WorkerBody {
    stream: ByteStream,
}

#[wasm_bindgen]
impl WorkerBody {
    /// The next piece as a `Uint8Array`, or `undefined` at the end of the body.
    pub async fn next(&mut self) -> Result<JsValue, JsValue> {
        use futures::StreamExt;
        match self.stream.next().await {
            Some(Ok(piece)) => Ok(Uint8Array::from(piece.as_slice()).into()),
            Some(Err(e)) => Err(JsValue::from_str(&e.to_string())),
            None => Ok(JsValue::UNDEFINED),
        }
    }
}

/// A JavaScript `ReadableStream` of bytes as a [`ByteStream`].
fn readable_to_stream(readable: JsValue) -> Result<ByteStream, JsValue> {
    let reader = js_sys::Reflect::get(&readable, &"getReader".into())?
        .dyn_into::<js_sys::Function>()?
        .call0(&readable)?;
    Ok(Box::pin(futures::stream::unfold(
        Some(reader),
        |state| async move {
            let reader = state?;
            let step = call(&reader, "read", &[]).await;
            let result = match step {
                Ok(r) => r,
                Err(e) => return Some((Err(e), None)),
            };
            if Reflect::get(&result, &"done".into())
                .ok()
                .and_then(|d| d.as_bool())
                .unwrap_or(true)
            {
                return None;
            }
            let value = Reflect::get(&result, &"value".into()).unwrap_or(JsValue::UNDEFINED);
            Some((Ok(Uint8Array::new(&value).to_vec()), Some(reader)))
        },
    )))
}

/// Stateless Cloudflare Worker entrypoint compiled to `wasm32-unknown-unknown`.
/// Connects to SurrealDB Cloud Earth (or any SurrealDB 3.x instance) over 1-RTT HTTP JSON-RPC
/// and dispatches `/api/*`, `/raw`, `/mcp`, `/dav/*`, and `/s3/*` through `SurrealFsGateway`.
#[wasm_bindgen]
pub async fn handle_worker_request(
    method: String,
    path_and_query: String,
    headers_json: String,
    body: Vec<u8>,
    body_stream: JsValue,
    env_json: String,
    blobs: JsValue,
) -> Result<JsValue, JsValue> {
    let env: WorkerEnv = serde_json::from_str(&env_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid worker env JSON: {}", e)))?;

    if env.surrealdb_url.trim().is_empty() {
        return Err(JsValue::from_str(
            "SURREALDB_URL secret/variable is required in Cloudflare Worker environment",
        ));
    }

    let ns = env
        .surrealdb_ns
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "surrealfs".to_string());
    let db = env
        .surrealdb_db
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "main".to_string());

    let (user, pass) = if let Some(tok) = env.surrealdb_token.filter(|s| !s.trim().is_empty()) {
        (None, Some(tok))
    } else {
        (env.surrealdb_user, env.surrealdb_pass)
    };

    let fs = SurrealFs::connect_http(ConnectOptions {
        url: env.surrealdb_url,
        ns,
        db,
        user,
        pass,
        caller: Some("root".to_string()),
    })
    .map_err(|e| JsValue::from_str(&format!("SurrealFs connect_http failed: {}", e)))?;
    let fs = if blobs.is_null() || blobs.is_undefined() {
        fs
    } else {
        fs.with_blob_store(Arc::new(R2BlobStore::new(blobs)))
    };

    let embedder = env
        .openai_api_key
        .filter(|k| !k.trim().is_empty())
        .map(|k| Embedder::new(k, env.openai_base_url, env.embed_model));

    let chat_cfg = env
        .anthropic_api_key
        .filter(|k| !k.trim().is_empty())
        .map(|k| ChatConfig::new(k, env.chat_model));

    let gateway = SurrealFsGateway::new(fs)
        .with_embedder(embedder)
        .with_chat(chat_cfg)
        .with_server_token(env.server_token);

    let headers: HashMap<String, String> = serde_json::from_str(&headers_json).unwrap_or_default();
    let mut req = HttpRequest::from_parts(&method, &path_and_query, headers, body);
    if !body_stream.is_null() && !body_stream.is_undefined() {
        req.body_stream = Some(BodyStream::new(readable_to_stream(body_stream)?));
    }
    let resp = gateway.handle(req).await;

    let headers = Object::new();
    for (k, v) in &resp.headers {
        Reflect::set(&headers, &k.as_str().into(), &v.as_str().into())?;
    }
    let out = Object::new();
    Reflect::set(&out, &"status".into(), &resp.status.into())?;
    Reflect::set(&out, &"headers".into(), &headers)?;
    Reflect::set(
        &out,
        &"body".into(),
        &Uint8Array::from(resp.body.as_slice()).into(),
    )?;
    if let Some(stream) = resp.stream.as_ref().and_then(|s| s.take()) {
        Reflect::set(&out, &"stream".into(), &WorkerBody { stream }.into())?;
    }
    Ok(out.into())
}
