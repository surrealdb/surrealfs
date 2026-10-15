use serde::Deserialize;
use serde_json::{Map, Value, json};
use surrealfs_core::{FileEntry, SurrealFs, SurrealFsError};

use crate::chat::{ChatConfig, handle_chat, handle_session};
use crate::embed::{Embedder, index_file_on_write, process_pending_jobs};
use crate::http::{HttpRequest, HttpResponse};

fn entry_json(e: &FileEntry) -> Value {
    json!({
        "path": e.path,
        "filename": e.filename,
        "is_folder": e.is_folder,
        "content_type": e.content_type,
        "size": e.size,
        "generation": e.generation,
        "updated_at": e.updated_at,
    })
}

fn error_status(err: &SurrealFsError) -> u16 {
    match err {
        SurrealFsError::NotFound(_) => 404,
        SurrealFsError::PermissionDenied(_) | SurrealFsError::LegalHold(_) => 403,
        SurrealFsError::AlreadyExists(_)
        | SurrealFsError::DirectoryNotEmpty(_)
        | SurrealFsError::Conflict(_)
        | SurrealFsError::Lock(_) => 409,
        SurrealFsError::InvalidPath(_)
        | SurrealFsError::IsADirectory(_)
        | SurrealFsError::InvalidArguments(_) => 400,
        _ => 500,
    }
}

fn err_response(err: SurrealFsError) -> HttpResponse {
    let status = error_status(&err);
    HttpResponse::json(status, &json!({ "error": err.to_string() }))
}

fn guess_content_type(path: &str, stored: Option<&str>) -> String {
    if let Some(ct) = stored.filter(|s| !s.is_empty() && *s != "application/octet-stream") {
        return ct.to_string();
    }
    let lower = path.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "text/markdown; charset=utf-8".to_string()
    } else if lower.ends_with(".json") {
        "application/json; charset=utf-8".to_string()
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8".to_string()
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8".to_string()
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") || lower.ends_with(".ts") {
        "text/javascript; charset=utf-8".to_string()
    } else if lower.ends_with(".png") {
        "image/png".to_string()
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg".to_string()
    } else if lower.ends_with(".gif") {
        "image/gif".to_string()
    } else if lower.ends_with(".webp") {
        "image/webp".to_string()
    } else if lower.ends_with(".svg") {
        "image/svg+xml".to_string()
    } else if lower.ends_with(".pdf") {
        "application/pdf".to_string()
    } else {
        "text/plain; charset=utf-8".to_string()
    }
}

#[derive(Deserialize)]
struct PutFileBody {
    path: String,
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct CreateFileBody {
    path: String,
    #[serde(default)]
    folder: bool,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct MoveCopyBody {
    src: String,
    dst: String,
    #[serde(default)]
    recursive: bool,
}

#[derive(Deserialize)]
struct RestoreBody {
    path: String,
    generation: u64,
}

#[derive(Deserialize)]
struct RpcRequestBody {
    sql: String,
    #[serde(default)]
    vars: Map<String, Value>,
    #[serde(default)]
    caller: Option<String>,
}

/// Handles `/raw?path=...` requests for raw file bytes.
pub async fn handle_raw(fs: &SurrealFs, req: &HttpRequest) -> HttpResponse {
    let params = req.query_params();
    let path = match params.get("path").filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => return HttpResponse::json(400, &json!({ "error": "path query param required" })),
    };

    let info = match fs.stat(path).await {
        Ok(Some(e)) if !e.is_folder => e,
        Ok(Some(_)) => {
            return HttpResponse::json(400, &json!({ "error": format!("Is a directory: {}", path) }));
        }
        Ok(None) => return HttpResponse::json(404, &json!({ "error": "Not found" })),
        Err(e) => return err_response(e),
    };

    match fs.read_bytes(path).await {
        Ok(bytes) => {
            let ct_opt = if info.content_type.is_empty() {
                None
            } else {
                Some(info.content_type.as_str())
            };
            let ct = guess_content_type(path, ct_opt);
            let mut resp = HttpResponse::with_bytes(200, bytes, &ct);
            resp.headers
                .insert("X-Content-Type-Options".to_string(), "nosniff".to_string());
            resp.headers
                .insert("Content-Security-Policy".to_string(), "sandbox".to_string());
            resp
        }
        Err(e) => err_response(e),
    }
}

/// Handles all `/api/*` endpoints.
pub async fn handle_api(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    chat_cfg: Option<&ChatConfig>,
    req: &HttpRequest,
) -> HttpResponse {
    if req.method == "OPTIONS" {
        return HttpResponse::empty(204);
    }

    let path = req.clean_path();
    let params = req.query_params();

    match (req.method.as_str(), path) {
        ("GET", "/api/health") => HttpResponse::json(
            200,
            &json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION"),
                "embeddings_enabled": embedder.is_some(),
                "chat_enabled": chat_cfg.is_some(),
            }),
        ),

        ("GET", "/api/tree") => match fs.ls_recursive("/").await {
            Ok(entries) => {
                let list: Vec<Value> = entries.iter().map(entry_json).collect();
                HttpResponse::json(200, &list)
            }
            Err(e) => err_response(e),
        },

        ("GET", "/api/stat") => {
            let Some(p) = params.get("path").filter(|s| !s.is_empty()) else {
                return HttpResponse::json(400, &json!({ "error": "path required" }));
            };
            match fs.stat(p).await {
                Ok(Some(e)) => HttpResponse::json(200, &entry_json(&e)),
                Ok(None) => HttpResponse::json(404, &json!({ "error": format!("Not found: {}", p) })),
                Err(e) => err_response(e),
            }
        }

        ("GET", "/api/file") => {
            let Some(p) = params.get("path").filter(|s| !s.is_empty()) else {
                return HttpResponse::json(400, &json!({ "error": "path required" }));
            };
            match fs.stat(p).await {
                Ok(Some(e)) => {
                    let mut obj = entry_json(&e);
                    if !e.is_folder {
                        if let Ok(text) = fs.read_text(p).await {
                            if let Some(m) = obj.as_object_mut() {
                                m.insert("content".to_string(), Value::String(text));
                            }
                        }
                    }
                    HttpResponse::json(200, &obj)
                }
                Ok(None) => HttpResponse::json(404, &json!({ "error": format!("Not found: {}", p) })),
                Err(e) => err_response(e),
            }
        }

        ("PUT", "/api/file") => {
            let body: PutFileBody = match serde_json::from_slice(&req.body) {
                Ok(b) => b,
                Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
            };
            match fs.write_text(&body.path, &body.content, None).await {
                Ok(entry) => {
                    let _ = index_file_on_write(fs, embedder, &entry.path).await;
                    HttpResponse::json(200, &entry_json(&entry))
                }
                Err(e) => err_response(e),
            }
        }

        ("POST", "/api/file") => {
            let body: CreateFileBody = match serde_json::from_slice(&req.body) {
                Ok(b) => b,
                Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
            };
            if let Ok(Some(_)) = fs.stat(&body.path).await {
                return HttpResponse::json(
                    409,
                    &json!({ "error": format!("File exists: {}", body.path) }),
                );
            }
            let res = if body.folder {
                fs.mkdir(&body.path, true).await
            } else {
                let text = body.content.as_deref().unwrap_or("");
                fs.write_text(&body.path, text, None).await
            };
            match res {
                Ok(entry) => {
                    if !entry.is_folder {
                        let _ = index_file_on_write(fs, embedder, &entry.path).await;
                    }
                    HttpResponse::json(200, &entry_json(&entry))
                }
                Err(e) => err_response(e),
            }
        }

        ("DELETE", "/api/file") => {
            let Some(p) = params.get("path").filter(|s| !s.is_empty()) else {
                return HttpResponse::json(400, &json!({ "error": "path required" }));
            };
            let recursive = params
                .get("recursive")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false);
            match fs.rm(p, recursive).await {
                Ok(()) => HttpResponse::json(200, &json!({ "removed": 1 })),
                Err(e) => err_response(e),
            }
        }

        ("POST", "/api/move") => {
            let body: MoveCopyBody = match serde_json::from_slice(&req.body) {
                Ok(b) => b,
                Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
            };
            match fs.mv(&body.src, &body.dst).await {
                Ok(entry) => HttpResponse::json(200, &entry_json(&entry)),
                Err(e) => err_response(e),
            }
        }

        ("POST", "/api/copy") => {
            let body: MoveCopyBody = match serde_json::from_slice(&req.body) {
                Ok(b) => b,
                Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
            };
            match fs.cp(&body.src, &body.dst, body.recursive).await {
                Ok(entry) => HttpResponse::json(200, &entry_json(&entry)),
                Err(e) => err_response(e),
            }
        }

        ("GET", "/api/search") => {
            let q = params
                .get("q")
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            if q.is_empty() {
                return HttpResponse::json(200, &json!({ "hybrid": false, "results": [] }));
            }
            let qvec = if let Some(emb) = embedder {
                emb.embed(&q).await.ok().filter(|v| !v.is_empty())
            } else {
                None
            };
            let hybrid = qvec.is_some();
            match fs.search_hybrid(&q, qvec.as_deref(), 20).await {
                Ok(hits) => {
                    let results: Vec<Value> = hits
                        .into_iter()
                        .map(|h| {
                            let mut obj = entry_json(&h.entry);
                            obj["snippet"] = json!(h.snippet);
                            obj["score"] = json!((h.score * 10000.0).round() / 10000.0);
                            obj
                        })
                        .collect();
                    HttpResponse::json(200, &json!({ "hybrid": hybrid, "results": results }))
                }
                Err(e) => err_response(e),
            }
        }

        ("GET", "/api/graph") => handle_graph(fs).await,

        ("GET", "/api/activity") => handle_activity(fs).await,

        ("GET", "/api/history") => {
            let Some(p) = params.get("path").filter(|s| !s.is_empty()) else {
                return HttpResponse::json(400, &json!({ "error": "path required" }));
            };
            match fs.history(p, Some(30)).await {
                Ok(versions) => HttpResponse::json(200, &versions),
                Err(e) => err_response(e),
            }
        }

        ("POST", "/api/restore") => {
            let body: RestoreBody = match serde_json::from_slice(&req.body) {
                Ok(b) => b,
                Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
            };
            match fs.restore(&body.path, body.generation).await {
                Ok(entry) => {
                    let _ = index_file_on_write(fs, embedder, &entry.path).await;
                    HttpResponse::json(200, &entry_json(&entry))
                }
                Err(e) => err_response(e),
            }
        }

        ("GET", "/api/session") => match handle_session(fs, params).await {
            Ok(resp) => resp,
            Err(e) => err_response(e),
        },

        ("POST", "/api/chat") => handle_chat(fs, chat_cfg, embedder, req).await,

        ("POST", "/api/index") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
            if let Some(p) = body.get("path").and_then(|v| v.as_str()) {
                match index_file_on_write(fs, embedder, p).await {
                    Ok(()) => HttpResponse::json(200, &json!({ "indexed": p })),
                    Err(e) => err_response(e),
                }
            } else {
                match process_pending_jobs(fs, embedder, "server-worker", 25).await {
                    Ok(count) => HttpResponse::json(200, &json!({ "completed_jobs": count })),
                    Err(e) => err_response(e),
                }
            }
        }

        ("POST", "/api/rpc") => handle_rpc(fs, embedder, req).await,

        _ => HttpResponse::json(404, &json!({ "error": format!("Unknown API route: {} {}", req.method, path) })),
    }
}

async fn handle_graph(fs: &SurrealFs) -> HttpResponse {
    let entries = match fs.ls_recursive("/").await {
        Ok(e) => e,
        Err(e) => return err_response(e),
    };
    let readable_paths: std::collections::HashSet<&str> =
        entries.iter().map(|e| e.path.as_str()).collect();

    let nodes: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "id": e.path,
                "label": e.filename,
                "path": e.path,
                "is_folder": e.is_folder,
                "size": e.size,
                "updated_at": e.updated_at,
                "content_type": e.content_type,
            })
        })
        .collect();

    let mut edges: Vec<Value> = Vec::new();
    for rel in &[
        "references",
        "supersedes",
        "derives_from",
        "implements",
        "links_to",
    ] {
        let sql = format!("SELECT in.path AS src, out.path AS dst FROM {};", rel);
        if let Ok(mut stmts) = fs.raw_query(&sql, Map::new()).await {
            if let Some(Ok(Value::Array(rows))) = stmts.pop() {
                for r in rows {
                    if let (Some(src), Some(dst)) = (
                        r.get("src").and_then(|v| v.as_str()),
                        r.get("dst").and_then(|v| v.as_str()),
                    ) {
                        if readable_paths.contains(src) && readable_paths.contains(dst) {
                            edges.push(json!({ "source": src, "target": dst, "relation": rel }));
                        }
                    }
                }
            }
        }
    }

    for e in &entries {
        if let Some(pos) = e.path.rfind('/') {
            let parent = if pos == 0 { "/" } else { &e.path[..pos] };
            if parent != "/" && readable_paths.contains(parent) {
                edges.push(json!({
                    "source": parent,
                    "target": e.path,
                    "relation": "contains",
                }));
            }
        }
    }

    HttpResponse::json(200, &json!({ "nodes": nodes, "edges": edges }))
}

async fn handle_activity(fs: &SurrealFs) -> HttpResponse {
    let readable_entries = fs.ls_recursive("/").await.unwrap_or_default();
    let readable_paths: std::collections::HashSet<&str> =
        readable_entries.iter().map(|e| e.path.as_str()).collect();
    let is_root = fs.caller().is_none() || fs.caller() == Some("root");

    let locks: Vec<_> = fs
        .list_locks()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|l| is_root || readable_paths.contains(l.path.as_str()))
        .collect();

    let mut vars = Map::new();
    vars.insert(
        "caller".to_string(),
        Value::String(fs.caller().unwrap_or("root").to_string()),
    );
    let recent_versions = match fs
        .raw_query(
            "SELECT path ?? file.path AS path, generation, author, created_at, size, op FROM file_version WHERE $caller = 'root' OR fn::sfs_can_read_grp(IF file.id IS NOT NONE THEN fn::sfs_gate(file.parent) ELSE gate END, owner, mode, $caller, file.group) ORDER BY created_at DESC LIMIT 20;",
            vars,
        )
        .await
    {
        Ok(mut stmts) => match stmts.pop() {
            Some(Ok(v)) => v,
            _ => json!([]),
        },
        Err(_) => json!([]),
    };

    HttpResponse::json(
        200,
        &json!({
            "locks": locks,
            "recent_versions": recent_versions,
        }),
    )
}

async fn handle_rpc(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    req: &HttpRequest,
) -> HttpResponse {
    let body: RpcRequestBody = match serde_json::from_slice(&req.body) {
        Ok(b) => b,
        Err(e) => return HttpResponse::json(400, &json!({ "error": e.to_string() })),
    };

    let target_fs = match body.caller.as_deref().filter(|c| !c.is_empty()) {
        Some(caller) if fs.caller().is_none() || fs.caller() == Some("root") => {
            fs.with_caller(caller)
        }
        _ => fs.clone(),
    };

    let mut vars = body.vars;
    if body.sql.contains("$caller") && !vars.contains_key("caller") {
        if let Some(c) = target_fs.caller() {
            vars.insert("caller".to_string(), Value::String(c.to_string()));
        }
    }

    let mutated_path = if body.sql.contains("fn::sfs_write")
        || body.sql.contains("fn::sfs_edit")
        || body.sql.contains("fn::sfs_append")
    {
        vars.get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    match target_fs.raw_query(&body.sql, vars).await {
        Ok(results) => {
            if let Some(ref p) = mutated_path {
                let _ = index_file_on_write(&target_fs, embedder, p).await;
            }
            let envelopes: Vec<Value> = results
                .into_iter()
                .map(|r| match r {
                    Ok(val) => json!({ "status": "OK", "result": val }),
                    Err(err) => json!({ "status": "ERR", "result": err.to_string() }),
                })
                .collect();
            HttpResponse::json(200, &json!({ "result": envelopes }))
        }
        Err(err) => HttpResponse::json(
            200,
            &json!({
                "result": [
                    { "status": "ERR", "result": err.to_string() }
                ]
            }),
        ),
    }
}
