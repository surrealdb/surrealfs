use serde_json::{Value, json};
use surrealfs_core::{Result, SurrealFs, SurrealFsError};

use crate::embed::{Embedder, index_file_on_write};
use crate::http::{HttpRequest, HttpResponse};

/// Returns the canonical JSON Schema tool specifications for all SurrealFS tools.
pub fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "ls",
            "description": "List files and directories at the given path.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (default '/')" }
                }
            }
        }),
        json!({
            "name": "cat",
            "description": "Read the UTF-8 text content of a file, optionally slicing by line range.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "offset": { "type": "integer", "description": "1-indexed start line" },
                    "limit": { "type": "integer", "description": "Maximum number of lines to return" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "stat",
            "description": "Return metadata for a file or folder (size, content_type, generation, timestamps).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "write_file",
            "description": "Create or overwrite a text file (automatically creating parent folders and indexing embeddings).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "content": { "type": "string", "description": "UTF-8 text content" }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "append_file",
            "description": "Append UTF-8 text to an existing or new file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "content": { "type": "string", "description": "Text chunk to append" }
                },
                "required": ["path", "content"]
            }
        }),
        json!({
            "name": "edit_file",
            "description": "Atomically replace an exact substring `old_string` with `new_string` inside a text file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "old_string": { "type": "string", "description": "Exact text to replace" },
                    "new_string": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_string", "new_string"]
            }
        }),
        json!({
            "name": "mkdir",
            "description": "Create a directory (and parents by default).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path" },
                    "parents": { "type": "boolean", "description": "Create parent directories (default true)" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "rm",
            "description": "Remove a file or directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Target path" },
                    "recursive": { "type": "boolean", "description": "Recursively delete non-empty folder" }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "mv",
            "description": "Move or rename a file or directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "src": { "type": "string", "description": "Source path" },
                    "dst": { "type": "string", "description": "Destination path" }
                },
                "required": ["src", "dst"]
            }
        }),
        json!({
            "name": "cp",
            "description": "Copy a file or directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "src": { "type": "string", "description": "Source path" },
                    "dst": { "type": "string", "description": "Destination path" },
                    "recursive": { "type": "boolean", "description": "Copy directories recursively" }
                },
                "required": ["src", "dst"]
            }
        }),
        json!({
            "name": "glob",
            "description": "Match file paths by glob pattern (e.g. '/docs/**/*.md').",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern" }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "grep",
            "description": "Search file contents by substring or regular expression.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Search pattern" },
                    "path": { "type": "string", "description": "Optional path prefix" },
                    "regex": { "type": "boolean", "description": "Treat pattern as regex" }
                },
                "required": ["pattern"]
            }
        }),
        json!({
            "name": "search_text",
            "description": "Full-text BM25 ranked search over files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max hits (default 10)" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "search_semantic",
            "description": "Semantic vector similarity search over files using server-side embeddings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query" },
                    "limit": { "type": "integer", "description": "Max hits (default 10)" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "search_hybrid",
            "description": "Hybrid BM25 full-text + vector RRF search over files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max hits (default 10)" }
                },
                "required": ["query"]
            }
        }),
    ]
}

/// Dispatches a tool call against `SurrealFs` and returns a JSON value result plus optional modified path.
pub async fn dispatch_tool_call(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    name: &str,
    args: &Value,
) -> Result<(Value, Option<String>)> {
    match name {
        "ls" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            let entries = fs.ls(path).await?;
            Ok((serde_json::to_value(entries)?, None))
        }
        "cat" | "read_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let text = fs.read_text(path).await?;
            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as usize;
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
            let lines: Vec<&str> = text.lines().collect();
            let start = (offset - 1).min(lines.len());
            let end = match limit {
                Some(l) => (start + l).min(lines.len()),
                None => lines.len(),
            };
            let sliced = lines[start..end].join("\n");
            Ok((json!({ "path": path, "content": sliced }), None))
        }
        "stat" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let entry = fs
                .stat(path)
                .await?
                .ok_or_else(|| SurrealFsError::NotFound(path.to_string()))?;
            Ok((serde_json::to_value(entry)?, None))
        }
        "write_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let entry = fs.write_text(path, content, None).await?;
            let _ = index_file_on_write(fs, embedder, &entry.path).await;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "append_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let content = args
                .get("content")
                .or_else(|| args.get("chunk"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let entry = fs.append_text(path, content).await?;
            let _ = index_file_on_write(fs, embedder, &entry.path).await;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "edit_file" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let old_s = args
                .get("old_string")
                .or_else(|| args.get("old"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::Other("Missing old_string".to_string()))?;
            let new_s = args
                .get("new_string")
                .or_else(|| args.get("new"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let entry = fs.edit_text(path, old_s, new_s, None).await?;
            let _ = index_file_on_write(fs, embedder, &entry.path).await;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "mkdir" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let parents = args
                .get("parents")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let entry = fs.mkdir(path, parents).await?;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "rm" => {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing path".to_string()))?;
            let recursive = args
                .get("recursive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            fs.rm(path, recursive).await?;
            Ok((json!({ "removed": path }), Some(path.to_string())))
        }
        "mv" => {
            let src = args
                .get("src")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing src".to_string()))?;
            let dst = args
                .get("dst")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing dst".to_string()))?;
            let entry = fs.mv(src, dst).await?;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "cp" => {
            let src = args
                .get("src")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing src".to_string()))?;
            let dst = args
                .get("dst")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::InvalidPath("Missing dst".to_string()))?;
            let recursive = args
                .get("recursive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let entry = fs.cp(src, dst, recursive).await?;
            Ok((serde_json::to_value(&entry)?, Some(entry.path)))
        }
        "glob" => {
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("/**/*");
            let re_str = surrealfs_core::glob_to_regex(pattern);
            let re = regex::Regex::new(&re_str)
                .map_err(|e| SurrealFsError::InvalidPath(e.to_string()))?;
            let all = fs.ls_recursive("/").await?;
            let matched: Vec<_> = all.into_iter().filter(|e| re.is_match(&e.path)).collect();
            Ok((serde_json::to_value(matched)?, None))
        }
        "grep" => {
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .ok_or_else(|| SurrealFsError::Other("Missing pattern".to_string()))?;
            let path = args.get("path").and_then(|v| v.as_str());
            let is_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
            let hits = fs.grep(pattern, path, is_regex).await?;
            Ok((serde_json::to_value(hits)?, None))
        }
        "search_text" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let hits = fs.search(query, limit).await?;
            Ok((serde_json::to_value(hits)?, None))
        }
        "search_semantic" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            if let Some(emb) = embedder {
                let qvec = emb.embed(query).await?;
                let hits = fs.search_semantic(&qvec, limit).await?;
                Ok((serde_json::to_value(hits)?, None))
            } else {
                Err(SurrealFsError::Other(
                    "OPENAI_API_KEY is not configured on the server for semantic search".to_string(),
                ))
            }
        }
        "search_hybrid" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            let qvec = if let Some(emb) = embedder {
                emb.embed(query).await.ok().filter(|v| !v.is_empty())
            } else {
                None
            };
            let hits = fs.search_hybrid(query, qvec.as_deref(), limit).await?;
            Ok((serde_json::to_value(hits)?, None))
        }
        other => Err(SurrealFsError::Other(format!("Unknown tool: {}", other))),
    }
}

/// Handles Streamable HTTP MCP requests (`POST /mcp` and `GET /mcp`).
pub async fn handle_mcp(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    req: &HttpRequest,
) -> HttpResponse {
    if req.method == "OPTIONS" {
        return HttpResponse::empty(204);
    }
    if req.method == "GET" {
        return HttpResponse::json(
            200,
            &json!({
                "name": "surrealfs-mcp",
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": "mcp-streamable-http",
                "tools": tool_specs().len()
            }),
        );
    }
    if req.method != "POST" {
        return HttpResponse::json(405, &json!({ "error": "Method not allowed" }));
    }

    let payload: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::json(
                400,
                &json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                }),
            );
        }
    };

    let id = payload.get("id").cloned().unwrap_or(Value::Null);
    let method = payload.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => HttpResponse::json(
            200,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": { "listChanged": false }
                    },
                    "serverInfo": {
                        "name": "surrealfs-server",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
        ),
        "notifications/initialized" => HttpResponse::empty(202),
        "ping" => HttpResponse::json(
            200,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
        ),
        "tools/list" => HttpResponse::json(
            200,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_specs()
                }
            }),
        ),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match dispatch_tool_call(fs, embedder, name, &arguments).await {
                Ok((val, _)) => {
                    let text =
                        serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
                    HttpResponse::json(
                        200,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [
                                    { "type": "text", "text": text }
                                ],
                                "isError": false
                            }
                        }),
                    )
                }
                Err(e) => HttpResponse::json(
                    200,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [
                                { "type": "text", "text": format!("Error: {}", e) }
                            ],
                            "isError": true
                        }
                    }),
                ),
            }
        }
        other => HttpResponse::json(
            200,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", other)
                }
            }),
        ),
    }
}
