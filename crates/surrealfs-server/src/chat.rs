use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use surrealfs_core::{Result, SurrealFs};

use crate::embed::Embedder;
use crate::http::{HttpRequest, HttpResponse};
use crate::mcp::{dispatch_tool_call, tool_specs};

pub const DEFAULT_CHAT_MODEL: &str = "claude-sonnet-4-5";
const SYSTEM_PROMPT: &str = "\
You are an assistant embedded in the SurrealFS browser. You have tools that \
read, write, edit, search and organise a virtual filesystem backed by SurrealDB. \
All paths start with `/`. Prefer `edit_file` over `write_file` when changing part \
of an existing file. Be concise.";

#[derive(Clone, Debug)]
pub struct ChatConfig {
    pub api_key: String,
    pub model: String,
}

impl ChatConfig {
    pub fn new(api_key: impl Into<String>, model: Option<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_CHAT_MODEL.to_string()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())?;
        let model = std::env::var("SURREALFS_CHAT_MODEL").ok();
        Some(Self::new(key, model))
    }
}

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
    path: Option<String>,
    session: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    messages: Vec<Value>,
}

fn slugify(s: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in s.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash && !slug.is_empty() {
            slug.push('-');
            prev_dash = true;
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "chat".to_string()
    } else {
        trimmed.to_string()
    }
}

fn summarize_tool(name: &str, input: &Value) -> String {
    let arg = input
        .get("path")
        .or_else(|| input.get("pattern"))
        .or_else(|| input.get("query"))
        .or_else(|| input.get("src"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if arg.is_empty() {
        name.to_string()
    } else {
        format!("{} {}", name, arg)
    }
}

fn is_mutating_tool(name: &str) -> bool {
    matches!(
        name,
        "write_file" | "append_file" | "edit_file" | "mkdir" | "rm" | "mv" | "cp"
    )
}

/// Converts stored Anthropic session messages into UI bubbles for `GET /api/session?path=...`.
pub fn session_bubbles(messages: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut pending_tools: Vec<String> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = msg.get("content");
        if role == "user" {
            if let Some(text) = content.and_then(|v| v.as_str()) {
                let cleaned = if let Some(idx) = text.find("]\n") {
                    if text.starts_with("[Currently viewing ") {
                        &text[idx + 2..]
                    } else {
                        text
                    }
                } else {
                    text
                };
                out.push(json!({ "who": "you", "text": cleaned }));
            }
        } else if role == "assistant" {
            if let Some(blocks) = content.and_then(|v| v.as_array()) {
                let mut texts = Vec::new();
                for b in blocks {
                    let btype = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if btype == "tool_use" {
                        let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let input = b.get("input").cloned().unwrap_or_else(|| json!({}));
                        pending_tools.push(summarize_tool(name, &input));
                    } else if btype == "text" {
                        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                            if !t.is_empty() {
                                texts.push(t.to_string());
                            }
                        }
                    }
                }
                if !texts.is_empty() {
                    let mut bubble = json!({
                        "who": "agent",
                        "text": texts.join("\n\n")
                    });
                    if !pending_tools.is_empty() {
                        bubble["tools"] = json!(std::mem::take(&mut pending_tools));
                    }
                    out.push(bubble);
                }
            }
        }
    }
    if !pending_tools.is_empty() {
        out.push(json!({
            "who": "agent",
            "text": "",
            "tools": pending_tools
        }));
    }
    out
}

/// Handles `GET /api/session` (both listing `/_sessions` and reading a specific session transcript).
pub async fn handle_session(
    fs: &SurrealFs,
    query_params: &std::collections::HashMap<String, String>,
) -> Result<HttpResponse> {
    if let Some(path) = query_params.get("path").filter(|p| !p.is_empty()) {
        let raw = fs.read_text(path).await?;
        let parsed: SessionFile =
            serde_json::from_str(&raw).unwrap_or(SessionFile { messages: vec![] });
        let bubbles = session_bubbles(&parsed.messages);
        return Ok(HttpResponse::json(200, &bubbles));
    }

    let entries = fs.ls_recursive("/_sessions").await.unwrap_or_default();
    let files: Vec<Value> = entries
        .into_iter()
        .filter(|e| !e.is_folder && e.path.ends_with(".json"))
        .map(|e| {
            json!({
                "path": e.path,
                "filename": e.filename,
                "is_folder": false,
                "content_type": e.content_type,
                "size": e.size,
                "updated_at": e.updated_at,
            })
        })
        .collect();
    Ok(HttpResponse::json(200, &files))
}

/// Handles `POST /api/chat` and returns an NDJSON response stream.
pub async fn handle_chat(
    fs: &SurrealFs,
    chat_cfg: Option<&ChatConfig>,
    embedder: Option<&Embedder>,
    req: &HttpRequest,
) -> HttpResponse {
    let mut ndjson_lines: Vec<String> = Vec::new();
    let mut emit = |val: Value| {
        if let Ok(line) = serde_json::to_string(&val) {
            ndjson_lines.push(format!("{}\n", line));
        }
    };

    let cfg = match chat_cfg {
        Some(c) => c,
        None => {
            emit(
                json!({ "error": "Set ANTHROPIC_API_KEY on the SurrealFS server to enable chat." }),
            );
            return ndjson_response(ndjson_lines);
        }
    };

    let body: ChatRequest = match serde_json::from_slice(&req.body) {
        Ok(b) => b,
        Err(e) => {
            emit(json!({ "error": format!("Invalid JSON body: {}", e) }));
            return ndjson_response(ndjson_lines);
        }
    };

    let user_msg = body.message.trim().to_string();
    if user_msg.is_empty() {
        return HttpResponse::json(400, &json!({ "error": "message required" }));
    }

    let (session_path, mut messages) = if let Some(s_path) =
        body.session.as_deref().filter(|s| !s.is_empty())
    {
        let existing = fs
            .read_text(s_path)
            .await
            .ok()
            .and_then(|raw| serde_json::from_str::<SessionFile>(&raw).ok())
            .map(|sf| sf.messages)
            .unwrap_or_default();
        (s_path.to_string(), existing)
    } else {
        let now = Utc::now();
        let path = format!(
            "/_sessions/{}/{}-{}.json",
            now.format("%Y-%m-%d"),
            now.format("%H%M%S"),
            slugify(&user_msg)
        );
        (path, Vec::new())
    };

    let prompt_content = match body.path.as_deref().filter(|p| !p.is_empty()) {
        Some(open_path) => format!("[Currently viewing {}]\n{}", open_path, user_msg),
        None => user_msg,
    };
    messages.push(json!({
        "role": "user",
        "content": prompt_content
    }));

    let anthropic_tools: Vec<Value> = tool_specs()
        .into_iter()
        .map(|t| {
            json!({
                "name": t["name"],
                "description": t["description"],
                "input_schema": t["inputSchema"],
            })
        })
        .collect();

    let client = Client::new();

    for _step in 0..10 {
        let req_payload = json!({
            "model": cfg.model,
            "max_tokens": 4096,
            "system": SYSTEM_PROMPT,
            "tools": anthropic_tools,
            "messages": messages,
        });

        let resp = match client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &cfg.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req_payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                emit(json!({ "error": format!("Anthropic request failed: {}", e) }));
                return ndjson_response(ndjson_lines);
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            emit(json!({ "error": format!("Anthropic API {}: {}", status, err_text) }));
            return ndjson_response(ndjson_lines);
        }

        let resp_json: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                emit(json!({ "error": format!("Invalid Anthropic JSON: {}", e) }));
                return ndjson_response(ndjson_lines);
            }
        };

        let content_blocks = resp_json
            .get("content")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut tool_uses = Vec::new();
        for block in &content_blocks {
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            emit(json!({ "delta": t }));
                        }
                    }
                }
                "tool_use" => {
                    tool_uses.push(block.clone());
                }
                _ => {}
            }
        }

        messages.push(json!({
            "role": "assistant",
            "content": content_blocks
        }));

        let stop_reason = resp_json
            .get("stop_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("end_turn");

        if stop_reason != "tool_use" || tool_uses.is_empty() {
            break;
        }

        let mut tool_results = Vec::new();
        for tu in tool_uses {
            let tu_id = tu
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = tu
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input = tu.get("input").cloned().unwrap_or_else(|| json!({}));

            emit(json!({
                "tool": summarize_tool(&name, &input),
                "mutates": is_mutating_tool(&name),
            }));

            let result_str = match dispatch_tool_call(fs, embedder, &name, &input).await {
                Ok((val, _)) => serde_json::to_string(&val).unwrap_or_else(|_| val.to_string()),
                Err(e) => format!("error: {}", e),
            };

            tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tu_id,
                "content": result_str
            }));
        }

        messages.push(json!({
            "role": "user",
            "content": tool_results
        }));
    }

    if let Ok(serialized) = serde_json::to_string(&SessionFile { messages }) {
        let _ = fs.write_text(&session_path, &serialized, None).await;
    }

    emit(json!({ "done": true, "session": session_path }));
    ndjson_response(ndjson_lines)
}

fn ndjson_response(lines: Vec<String>) -> HttpResponse {
    let mut resp =
        HttpResponse::with_bytes(200, lines.concat().into_bytes(), "application/x-ndjson");
    resp.headers
        .insert("Cache-Control".to_string(), "no-cache".to_string());
    resp
}
