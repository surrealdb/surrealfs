use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use surrealfs_core::{ConnectOptions, SurrealFs};
use wasm_bindgen::prelude::*;

use crate::chat::ChatConfig;
use crate::embed::Embedder;
use crate::gateway::SurrealFsGateway;
use crate::http::{HttpHandler, HttpRequest};

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

#[derive(Serialize)]
struct WorkerResponseEnvelope {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
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
    env_json: String,
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
    let req = HttpRequest::from_parts(&method, &path_and_query, headers, body);
    let resp = gateway.handle(req).await;

    let envelope = WorkerResponseEnvelope {
        status: resp.status,
        headers: resp.headers,
        body: resp.body,
    };

    let serialized = serde_json::to_string(&envelope)
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize response: {}", e)))?;
    Ok(JsValue::from_str(&serialized))
}
