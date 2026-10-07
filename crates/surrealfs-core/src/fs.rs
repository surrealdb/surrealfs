//! Core async SurrealFs implementation for Rust.

use crate::chunking::{chunk_data, decompress_chunk, FastCdcConfig};
use crate::crdt::CrdtDoc;
use crate::errors::{Result, SurrealFsError};
use crate::models::{
    CodeSymbol, EntityRecord, FileEntry, FileLock, FileVersion, FolderDigest, GrepMatch,
    MailboxMessage, PackResult, PackedBlock, PipelineJob, SearchHit, SectionHit, TableRow,
    UploadSession, UsageStats, WorkspaceDiff,
};
use crate::paths::normalize_path;
use crate::understanding::{
    compute_simhash, parse_tabular_records, simhash_similarity, ExtractedEntity,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::types::Value as SValue;
use surrealdb::Surreal;

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub url: String,
    pub user: Option<String>,
    pub pass: Option<String>,
    pub ns: String,
    pub db: String,
    pub caller: Option<String>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            url: "ws://127.0.0.1:8000".to_string(),
            user: Some("root".to_string()),
            pass: Some("root".to_string()),
            ns: "test".to_string(),
            db: "test".to_string(),
            caller: None,
        }
    }
}

fn value_to_serde<T: DeserializeOwned>(v: SValue) -> Result<T> {
    let json_val = v.into_json_value();
    serde_json::from_value(json_val).map_err(|e| SurrealFsError::Database(e.to_string()))
}

fn extract_bytes(val: Option<&Value>) -> Option<Vec<u8>> {
    let v = val?;
    if let Some(arr) = v.as_array() {
        return Some(
            arr.iter()
                .filter_map(|x| x.as_u64().map(|b| b as u8))
                .collect(),
        );
    }
    if let Some(s) = v.as_str() {
        use base64::Engine;
        if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(s) {
            return Some(b);
        }
        return Some(s.as_bytes().to_vec());
    }
    None
}

#[derive(Clone)]
pub struct SurrealFs {
    db: Surreal<Client>,
    caller: Option<String>,
}

impl SurrealFs {
    /// Connect to a remote SurrealDB instance via WebSocket.
    pub async fn connect(opts: ConnectOptions) -> Result<Self> {
        let endpoint = opts
            .url
            .trim_start_matches("ws://")
            .trim_start_matches("wss://");
        let db = Surreal::new::<Ws>(endpoint).await?;

        if let (Some(user), Some(pass)) = (&opts.user, &opts.pass) {
            db.signin(Root {
                username: user.clone(),
                password: pass.clone(),
            })
            .await?;
        }

        db.use_ns(&opts.ns).use_db(&opts.db).await?;

        Ok(Self {
            db,
            caller: opts.caller.or(opts.user),
        })
    }

    /// Return underlying Surreal client reference.
    pub fn client(&self) -> &Surreal<Client> {
        &self.db
    }

    /// Stat a file or directory.
    pub async fn stat(&self, path: &str) -> Result<Option<FileEntry>> {
        let norm = normalize_path(path);
        let mut res = match self
            .db
            .query("RETURN fn::sfs_stat($path);")
            .bind(("path", norm))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let sfe: SurrealFsError = e.into();
                if let SurrealFsError::NotFound(_) = sfe {
                    return Ok(None);
                }
                return Err(sfe);
            }
        };

        match res.take::<Option<SValue>>(0usize) {
            Ok(entry) => entry.map(value_to_serde).transpose(),
            Err(e) => {
                let sfe: SurrealFsError = e.into();
                if let SurrealFsError::NotFound(_) = sfe {
                    Ok(None)
                } else {
                    Err(sfe)
                }
            }
        }
    }

    /// Check if a path exists.
    pub async fn exists(&self, path: &str) -> Result<bool> {
        let s = self.stat(path).await?;
        Ok(s.is_some())
    }

    /// Read text content of a file.
    pub async fn read_text(&self, path: &str) -> Result<String> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_read($path, $caller);")
            .bind(("path", norm))
            .bind(("caller", self.caller.clone()))
            .await?;
        let content: Option<String> = res.take(0usize)?;
        content.ok_or_else(|| SurrealFsError::NotFound(format!("File not found: {}", path)))
    }

    /// Read raw bytes of a file.
    pub async fn read_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_read($path, $caller);")
            .bind(("path", norm.clone()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let val: Option<serde_json::Value> = res.take(0usize)?;
        let v = val.ok_or_else(|| SurrealFsError::NotFound(format!("File not found: {}", path)))?;
        if let Some(text) = v.as_str() {
            return Ok(text.as_bytes().to_vec());
        }
        if let Some(bytes) = extract_bytes(Some(&v)) {
            return Ok(bytes);
        }
        Ok(Vec::new())
    }

    /// Write raw bytes to a file.
    pub async fn write_bytes(
        &self,
        path: &str,
        data: &[u8],
        content_type: Option<&str>,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        if let Ok(text) = std::str::from_utf8(data) {
            return self.write_text(&norm, text, None).await;
        }
        let mime = content_type.unwrap_or("application/octet-stream");
        let mut res = self
            .db
            .query("LET $id = fn::sfs_resolve($path); IF $id IS NOT NONE { UPDATE $id SET file = <bytes>$data, content = NONE, content_type = $mime, updated_at = time::now(); RETURN fn::sfs_stat($path); }; LET $parent_id = fn::sfs_ensure_parents($path, $caller); LET $raw = string::split($path, '/'); LET $segments = array::filter($raw, |$v| string::len($v) > 0); LET $filename = $segments[array::len($segments) - 1]; LET $created = (CREATE file CONTENT { filename: $filename, parent: $parent_id, file: <bytes>$data, content_type: $mime }); RETURN fn::sfs_stat($path);")
            .bind(("path", norm.clone()))
            .bind(("data", data.to_vec()))
            .bind(("mime", mime.to_string()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(res.num_statements() - 1)?;
        let val =
            entry.ok_or_else(|| SurrealFsError::Database(format!("Failed to write: {}", norm)))?;
        value_to_serde(val)
    }

    /// Write or overwrite text file content.
    pub async fn write_text(
        &self,
        path: &str,
        content: &str,
        if_generation: Option<u64>,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        if let Ok(Some(existing)) = self.stat(&norm).await {
            if existing.crdt {
                let (mut doc, next_seq, base_gen) = self.load_crdt_doc(&norm).await?;
                let (delta, updated) = doc.apply_replace(content)?;
                return self
                    .apply_crdt_mutation(&norm, delta, &updated, next_seq, base_gen)
                    .await;
            }
        }
        let mut res = self
            .db
            .query("RETURN fn::sfs_write($path, $content, $if_gen, true, $caller);")
            .bind(("path", norm.clone()))
            .bind(("content", content.to_string()))
            .bind(("if_gen", if_generation))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val =
            entry.ok_or_else(|| SurrealFsError::Database(format!("Failed to write: {}", norm)))?;
        value_to_serde(val)
    }

    /// Append chunk to text file.
    pub async fn append_text(&self, path: &str, chunk: &str) -> Result<FileEntry> {
        let norm = normalize_path(path);
        if let Ok(Some(existing)) = self.stat(&norm).await {
            if existing.crdt {
                let (mut doc, next_seq, base_gen) = self.load_crdt_doc(&norm).await?;
                let (delta, updated) = doc.apply_append(chunk)?;
                return self
                    .apply_crdt_mutation(&norm, delta, &updated, next_seq, base_gen)
                    .await;
            }
        }
        let mut res = self
            .db
            .query("RETURN fn::sfs_append($path, $chunk, NONE, $caller);")
            .bind(("path", norm.clone()))
            .bind(("chunk", chunk.to_string()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val =
            entry.ok_or_else(|| SurrealFsError::Database(format!("Failed to append: {}", norm)))?;
        value_to_serde(val)
    }

    /// Atomically replace `old` substring with `new` in text file.
    pub async fn edit_text(
        &self,
        path: &str,
        old: &str,
        new: &str,
        if_generation: Option<u64>,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        if let Ok(Some(existing)) = self.stat(&norm).await {
            if existing.crdt {
                let (mut doc, next_seq, base_gen) = self.load_crdt_doc(&norm).await?;
                let (delta, updated) = doc.apply_edit(old, new)?;
                return self
                    .apply_crdt_mutation(&norm, delta, &updated, next_seq, base_gen)
                    .await;
            }
        }
        let mut res = self
            .db
            .query("RETURN fn::sfs_edit($path, $old, $new, $if_gen, $caller);")
            .bind(("path", norm.clone()))
            .bind(("old", old.to_string()))
            .bind(("new", new.to_string()))
            .bind(("if_gen", if_generation))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val =
            entry.ok_or_else(|| SurrealFsError::Database(format!("Failed to edit: {}", norm)))?;
        value_to_serde(val)
    }

    /// Create directory.
    pub async fn mkdir(&self, path: &str, recursive: bool) -> Result<FileEntry> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_mkdir($path, $parents, $caller);")
            .bind(("path", norm.clone()))
            .bind(("parents", recursive))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val =
            entry.ok_or_else(|| SurrealFsError::Database(format!("Failed to mkdir: {}", norm)))?;
        value_to_serde(val)
    }

    /// List directory contents.
    pub async fn ls(&self, path: &str) -> Result<Vec<FileEntry>> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_ls($path);")
            .bind(("path", norm))
            .await?;
        let entries: Vec<SValue> = res.take(0usize)?;
        entries.into_iter().map(value_to_serde).collect()
    }

    /// Delete file or directory.
    pub async fn rm(&self, path: &str, recursive: bool) -> Result<()> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_rm($path, $recursive, $caller);")
            .bind(("path", norm))
            .bind(("recursive", recursive))
            .bind(("caller", self.caller.clone()))
            .await?;
        let _res: Option<bool> = res.take(0usize)?;
        Ok(())
    }

    /// Move / rename file or directory.
    pub async fn mv(&self, src: &str, dst: &str) -> Result<FileEntry> {
        let norm_src = normalize_path(src);
        let norm_dst = normalize_path(dst);
        let mut res = self
            .db
            .query("RETURN fn::sfs_mv($src, $dst, $caller);")
            .bind(("src", norm_src))
            .bind(("dst", norm_dst.clone()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val = entry
            .ok_or_else(|| SurrealFsError::Database(format!("Failed to mv to: {}", norm_dst)))?;
        value_to_serde(val)
    }

    /// Copy file or directory.
    pub async fn cp(&self, src: &str, dst: &str, recursive: bool) -> Result<FileEntry> {
        let norm_src = normalize_path(src);
        let norm_dst = normalize_path(dst);
        let mut res = self
            .db
            .query("RETURN fn::sfs_cp($src, $dst, $recursive, $caller);")
            .bind(("src", norm_src))
            .bind(("dst", norm_dst.clone()))
            .bind(("recursive", recursive))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val = entry
            .ok_or_else(|| SurrealFsError::Database(format!("Failed to cp to: {}", norm_dst)))?;
        value_to_serde(val)
    }

    /// Retrieve generation history for a file.
    pub async fn history(&self, path: &str, limit: Option<usize>) -> Result<Vec<FileVersion>> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_history($path, $limit);")
            .bind(("path", norm))
            .bind(("limit", limit.unwrap_or(20) as i64))
            .await?;
        let list: Vec<SValue> = res.take(0usize)?;
        list.into_iter().map(value_to_serde).collect()
    }

    /// Restore file to previous generation.
    pub async fn restore(&self, path: &str, generation: u64) -> Result<FileEntry> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_restore($path, $gen, $caller);")
            .bind(("path", norm.clone()))
            .bind(("gen", generation as i64))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry: Option<SValue> = res.take(0usize)?;
        let val = entry
            .ok_or_else(|| SurrealFsError::Database(format!("Failed to restore: {}", norm)))?;
        value_to_serde(val)
    }

    /// Acquire advisory swarm lock.
    pub async fn acquire_lock(
        &self,
        path: &str,
        ttl_seconds: u64,
        reason: &str,
        holder: &str,
    ) -> Result<FileLock> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_acquire_lock($path, $holder, $ttl, $reason);")
            .bind(("path", norm.clone()))
            .bind(("holder", holder.to_string()))
            .bind(("ttl", ttl_seconds as i64))
            .bind(("reason", reason.to_string()))
            .await?;
        let lock: Option<SValue> = res.take(0usize)?;
        let val =
            lock.ok_or_else(|| SurrealFsError::Lock(format!("Lock conflict on: {}", norm)))?;
        value_to_serde(val)
    }

    /// Release advisory swarm lock.
    pub async fn release_lock(&self, path: &str, holder: &str) -> Result<()> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_release_lock($path, $holder);")
            .bind(("path", norm))
            .bind(("holder", holder.to_string()))
            .await?;
        let _released: Option<bool> = res.take(0usize)?;
        Ok(())
    }

    /// List active swarm locks.
    pub async fn list_locks(&self) -> Result<Vec<FileLock>> {
        let mut res = self
            .db
            .query("SELECT * FROM file_lock WHERE expires_at > time::now();")
            .await?;
        let locks: Vec<SValue> = res.take(0usize)?;
        locks.into_iter().map(value_to_serde).collect()
    }

    /// Fork a zero-copy workspace.
    pub async fn fork_workspace(&self, src: &str, dst: &str, owner: &str) -> Result<Value> {
        let mut res = self
            .db
            .query("RETURN fn::sfs_fork_workspace($src, $dst, $owner);")
            .bind(("src", src.to_string()))
            .bind(("dst", dst.to_string()))
            .bind(("owner", owner.to_string()))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let v = val.ok_or_else(|| SurrealFsError::Database("Workspace fork failed".to_string()))?;
        value_to_serde(v)
    }

    /// Diff a workspace against its base.
    pub async fn diff_workspace(&self, branch: &str, owner: &str) -> Result<Vec<WorkspaceDiff>> {
        let mut res = self
            .db
            .query("RETURN fn::sfs_diff_workspace($branch, $owner);")
            .bind(("branch", branch.to_string()))
            .bind(("owner", owner.to_string()))
            .await?;
        let diffs: Vec<SValue> = res.take(0usize)?;
        diffs.into_iter().map(value_to_serde).collect()
    }

    /// Merge workspace back into parent.
    pub async fn merge_workspace(&self, branch: &str, target: &str, owner: &str) -> Result<Value> {
        let mut res = self
            .db
            .query("RETURN fn::sfs_merge_workspace($branch, $target, $owner);")
            .bind(("branch", branch.to_string()))
            .bind(("target", target.to_string()))
            .bind(("owner", owner.to_string()))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let v =
            val.ok_or_else(|| SurrealFsError::Database("Workspace merge failed".to_string()))?;
        value_to_serde(v)
    }

    /// Send message to agent mailbox.
    pub async fn send_mailbox(
        &self,
        agent_id: &str,
        op: &str,
        payload: Value,
        priority: i64,
    ) -> Result<String> {
        let payload_str = payload.to_string();
        let mut res = self
            .db
            .query(
                "CREATE ONLY agent_mailbox CONTENT {
                agent_id: $agent,
                op: $op,
                payload: $payload,
                priority: $priority,
                state: 'pending',
                created_at: time::now()
            };",
            )
            .bind(("agent", agent_id.to_string()))
            .bind(("op", op.to_string()))
            .bind(("payload", payload_str))
            .bind(("priority", priority))
            .await?;
        let msg: Option<SValue> = res.take(0usize)?;
        if let Some(m) = msg {
            let parsed: MailboxMessage = value_to_serde(m)?;
            Ok(parsed.id)
        } else {
            Ok(String::new())
        }
    }

    /// Claim next mailbox task for agent worker.
    pub async fn claim_mailbox(
        &self,
        agent_id: &str,
        worker_id: &str,
        lease_seconds: u64,
    ) -> Result<Option<MailboxMessage>> {
        let sql = r#"
            LET $target = (
                SELECT id, priority, created_at FROM agent_mailbox 
                WHERE agent_id = $agent 
                  AND (state = 'pending' OR (state = 'processing' AND lease_expires_at < time::now()))
                ORDER BY priority DESC, created_at ASC 
                LIMIT 1
            )[0].id;

            IF $target != NONE {
                UPDATE $target SET 
                    state = 'processing',
                    worker_id = $worker,
                    claimed_at = time::now(),
                    lease_expires_at = time::now() + type::duration(string::concat($ttl, 's'));
                SELECT * FROM ONLY $target;
            } ELSE {
                RETURN NONE;
            };
        "#;
        let mut res = self
            .db
            .query(sql)
            .bind(("agent", agent_id.to_string()))
            .bind(("worker", worker_id.to_string()))
            .bind(("ttl", lease_seconds))
            .await?;
        let msg: Option<SValue> = res.take(1usize)?;
        msg.map(value_to_serde).transpose()
    }

    /// Mark mailbox task complete.
    pub async fn complete_mailbox(&self, message_id: &str, result: Value) -> Result<()> {
        let sql = r#"
            UPDATE type::record($msg_id) SET 
                state = 'completed',
                completed_at = time::now(),
                result = $res;
        "#;
        self.db
            .query(sql)
            .bind(("msg_id", message_id.to_string()))
            .bind(("res", result.to_string()))
            .await?;
        Ok(())
    }

    /// Grep text or regex across files.
    pub async fn grep(
        &self,
        pattern: &str,
        path_prefix: Option<&str>,
        is_regex: bool,
    ) -> Result<Vec<GrepMatch>> {
        let prefix = path_prefix.map(normalize_path).unwrap_or("/".to_string());
        let mut res = self
            .db
            .query(
                "SELECT path, content FROM file WHERE is_folder = false AND content != NONE LIMIT 1000;",
            )
            .await?;
        let rows: Vec<SValue> = res.take(0usize)?;
        let mut matches = Vec::new();

        let regex = if is_regex {
            Some(regex::Regex::new(pattern).map_err(|e| SurrealFsError::Other(e.to_string()))?)
        } else {
            None
        };

        for val in rows {
            let row: Value = value_to_serde(val)?;
            if let (Some(p), Some(c)) = (
                row.get("path").and_then(|v| v.as_str()),
                row.get("content").and_then(|v| v.as_str()),
            ) {
                if !p.starts_with(&prefix) && prefix != "/" {
                    continue;
                }
                for (idx, line) in c.lines().enumerate() {
                    let matched = if let Some(ref re) = regex {
                        re.is_match(line)
                    } else {
                        line.contains(pattern)
                    };
                    if matched {
                        matches.push(GrepMatch {
                            path: p.to_string(),
                            line_number: idx + 1,
                            line_text: line.to_string(),
                        });
                    }
                }
            }
        }

        Ok(matches)
    }

    /// Search files with ranked query.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let mut res = self
            .db
            .query("RETURN fn::sfs_search_text($q, $limit);")
            .bind(("q", query.to_string()))
            .bind(("limit", limit as i64))
            .await?;
        let hits: Vec<SValue> = res.take(0usize)?;
        let mut out = Vec::new();

        for hit_val in hits {
            let hit: Value = value_to_serde(hit_val)?;
            if let Ok(entry) = serde_json::from_value::<FileEntry>(hit.clone()) {
                let score = hit.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
                let snippet = hit
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(SearchHit {
                    entry,
                    score,
                    snippet,
                });
            }
        }

        Ok(out)
    }

    /// Search fine-grained file sections using vector cosine similarity.
    pub async fn search_sections(&self, vector: &[f32], limit: usize) -> Result<Vec<SectionHit>> {
        let sql = r#"
            SELECT 
                file_id.path AS path,
                heading,
                line_start,
                line_end,
                content,
                vector::similarity::cosine(embedding, $vec) AS score
            FROM file_section
            WHERE embedding <|80, COSINE|> $vec
            ORDER BY score DESC
            LIMIT $limit;
        "#;
        let mut res = self
            .db
            .query(sql)
            .bind(("vec", vector.to_vec()))
            .bind(("limit", limit as i64))
            .await?;
        let rows: Vec<SValue> = res.take(0usize)?;
        let mut out = Vec::new();
        for r in rows {
            let hit: SectionHit = value_to_serde(r)?;
            out.push(hit);
        }
        Ok(out)
    }

    /// Enable collaborative Yjs CRDT mode on a text file.
    pub async fn enable_crdt(&self, path: &str) -> Result<()> {
        let norm = normalize_path(path);
        let existing = self.stat(&norm).await?.ok_or_else(|| {
            SurrealFsError::NotFound(format!("File not found to enable CRDT: {}", path))
        })?;
        if existing.crdt {
            return Ok(());
        }
        let content = self.read_text(&norm).await.unwrap_or_default();
        let (_doc, init_update) = CrdtDoc::init(&content);

        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                CREATE file_crdt_snapshot CONTENT {
                    file_id: $file_id,
                    upto: 1,
                    state: <bytes>$state,
                    author: $author
                };
                CREATE file_crdt_update CONTENT {
                    file_id: $file_id,
                    seq: 1,
                    update_bytes: <bytes>$state,
                    author: $author,
                    base_gen: $base_gen
                };
                UPDATE $file_id SET crdt = true;
                RETURN true;
            };
        "#;
        self.db
            .query(sql)
            .bind(("path", norm))
            .bind(("state", init_update))
            .bind(("author", self.caller.clone()))
            .bind(("base_gen", existing.generation as i64))
            .await?;
        Ok(())
    }

    /// Compact CRDT updates into a single snapshot.
    pub async fn compact_crdt(&self, path: &str) -> Result<()> {
        let norm = normalize_path(path);
        let existing = self.stat(&norm).await?.ok_or_else(|| {
            SurrealFsError::NotFound(format!("File not found to compact CRDT: {}", path))
        })?;
        if !existing.crdt {
            return Ok(());
        }
        let (doc, next_seq, _) = self.load_crdt_doc(&norm).await?;
        let upto = next_seq - 1;
        let snapshot = doc.get_snapshot();

        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                UPSERT file_crdt_snapshot CONTENT {
                    file_id: $file_id,
                    upto: $upto,
                    state: <bytes>$snapshot,
                    author: $author
                } WHERE file_id = $file_id;
                DELETE file_crdt_update WHERE file_id = $file_id AND seq <= $upto;
                RETURN true;
            };
        "#;
        self.db
            .query(sql)
            .bind(("path", norm))
            .bind(("upto", upto))
            .bind(("snapshot", snapshot))
            .bind(("author", self.caller.clone()))
            .await?;
        Ok(())
    }

    async fn load_crdt_doc(&self, path: &str) -> Result<(CrdtDoc, i64, u64)> {
        let norm = normalize_path(path);
        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                LET $stat = (SELECT generation, content FROM ONLY $file_id);
                LET $snap = (SELECT upto, state FROM ONLY file_crdt_snapshot WHERE file_id = $file_id LIMIT 1);
                LET $upto = IF $snap.upto IS NOT NONE { $snap.upto } ELSE { 0 };
                LET $updates = (
                    SELECT seq, update_bytes FROM file_crdt_update 
                    WHERE file_id = $file_id AND seq > $upto 
                    ORDER BY seq ASC
                );
                RETURN {
                    gen: $stat.generation,
                    content: $stat.content,
                    upto: $upto,
                    snap_state: $snap.state,
                    updates: $updates
                };
            };
        "#;
        let mut res = self.db.query(sql).bind(("path", norm)).await?;
        let val: Option<SValue> = res.take(0usize)?;
        let obj: Value = val.map(value_to_serde).transpose()?.unwrap_or(Value::Null);

        let snap_state = extract_bytes(obj.get("snap_state"));

        let mut update_blobs = Vec::new();
        let mut max_seq = obj.get("upto").and_then(|v| v.as_i64()).unwrap_or(0);

        if let Some(updates) = obj.get("updates").and_then(|v| v.as_array()) {
            for u in updates {
                if let Some(s) = u.get("seq").and_then(|v| v.as_i64()) {
                    if s > max_seq {
                        max_seq = s;
                    }
                }
                if let Some(bytes) = extract_bytes(u.get("update_bytes")) {
                    update_blobs.push(bytes);
                }
            }
        }

        let doc = CrdtDoc::load(snap_state.as_deref(), &update_blobs)?;
        let gen = obj.get("gen").and_then(|v| v.as_u64()).unwrap_or(1);
        Ok((doc, max_seq + 1, gen))
    }

    async fn apply_crdt_mutation(
        &self,
        path: &str,
        delta: Vec<u8>,
        new_content: &str,
        seq: i64,
        base_gen: u64,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                CREATE file_crdt_update CONTENT {
                    file_id: $file_id,
                    seq: $seq,
                    update_bytes: <bytes>$delta,
                    author: $author,
                    base_gen: $base_gen
                };
                UPDATE $file_id SET content = $content, updated_at = time::now();
                RETURN fn::sfs_stat($path);
            };
        "#;
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("delta", delta))
            .bind(("seq", seq))
            .bind(("author", self.caller.clone()))
            .bind(("base_gen", base_gen as i64))
            .bind(("content", new_content.to_string()))
            .await?;
        let entry_val: Option<SValue> = res.take(0usize)?;
        let v = entry_val
            .ok_or_else(|| SurrealFsError::Database("Failed to apply CRDT mutation".to_string()))?;
        value_to_serde(v)
    }

    /// Upload a file using FastCDC content-addressed chunking, deduplication,
    /// and transparent zstd compression.
    pub async fn upload_file(
        &self,
        path: &str,
        data: &[u8],
        if_generation: Option<u64>,
        config: Option<FastCdcConfig>,
    ) -> Result<FileEntry> {
        let norm = normalize_path(path);
        let chunks = chunk_data(data, config);
        if chunks.is_empty() {
            return self.write_bytes(&norm, &[], None).await;
        }

        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
        let total_size = data.len() as i64;

        // Step 1: Begin upload session and identify missing chunks
        let begin_sql = "RETURN fn::sfs_upload_begin($path, $size, $chunk_ids, $caller);";
        let mut begin_res = self
            .db
            .query(begin_sql)
            .bind(("path", norm.clone()))
            .bind(("size", total_size))
            .bind(("chunk_ids", chunk_ids.clone()))
            .bind(("caller", self.caller.clone()))
            .await?;
        let session_val: Option<SValue> = begin_res.take(0usize)?;
        let s_val = session_val.ok_or_else(|| {
            SurrealFsError::Database("Empty response from fn::sfs_upload_begin".to_string())
        })?;
        let session: UploadSession = value_to_serde(s_val)?;

        // Step 2: Upload missing chunks
        let missing_set: std::collections::HashSet<&str> =
            session.missing_chunks.iter().map(|s| s.as_str()).collect();

        let mut uploaded = std::collections::HashSet::new();
        for chunk in &chunks {
            if missing_set.contains(chunk.chunk_id.as_str()) && !uploaded.contains(&chunk.chunk_id)
            {
                uploaded.insert(chunk.chunk_id.clone());
                let chunk_data = chunk.data.as_ref().unwrap();
                let upload_chunk_sql = r#"
                    RETURN fn::sfs_upload_chunk(
                        $upload_id,
                        $chunk_id,
                        $uncompressed_size,
                        $stored_size,
                        $codec,
                        <bytes>$data,
                        $caller
                    );
                "#;
                self.db
                    .query(upload_chunk_sql)
                    .bind(("upload_id", session.upload_id.clone()))
                    .bind(("chunk_id", chunk.chunk_id.clone()))
                    .bind(("uncompressed_size", chunk.uncompressed_size as i64))
                    .bind(("stored_size", chunk.stored_size as i64))
                    .bind(("codec", chunk.codec.clone()))
                    .bind(("data", chunk_data.clone()))
                    .bind(("caller", self.caller.clone()))
                    .await?;
            }
        }

        // Step 3: Commit upload session
        let offsets: Vec<i64> = chunks.iter().map(|c| c.offset as i64).collect();
        let lengths: Vec<i64> = chunks.iter().map(|c| c.length as i64).collect();
        let commit_sql = r#"
            RETURN fn::sfs_upload_commit(
                $upload_id,
                $if_generation,
                $offsets,
                $lengths,
                $caller
            );
        "#;
        let mut commit_res = self
            .db
            .query(commit_sql)
            .bind(("upload_id", session.upload_id))
            .bind(("if_generation", if_generation.map(|g| g as i64)))
            .bind(("offsets", offsets))
            .bind(("lengths", lengths))
            .bind(("caller", self.caller.clone()))
            .await?;
        let entry_val: Option<SValue> = commit_res.take(0usize)?;
        let v = entry_val.ok_or_else(|| {
            SurrealFsError::Database("Empty response from fn::sfs_upload_commit".to_string())
        })?;
        value_to_serde(v)
    }

    /// Read an arbitrary byte range from a file, efficiently streaming only covering chunks.
    pub async fn read_range(&self, path: &str, offset: u64, length: u64) -> Result<Vec<u8>> {
        let norm = normalize_path(path);
        let sql = "RETURN fn::sfs_read_bytes_range($path, $offset, $length, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("offset", offset as i64))
            .bind(("length", length as i64))
            .bind(("caller", self.caller.clone()))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let s_val = val.ok_or_else(|| {
            SurrealFsError::NotFound(format!("File not found for read_range: {}", path))
        })?;

        let json_val: Value = value_to_serde(s_val)?;
        let chunked = json_val
            .get("chunked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !chunked {
            let data_bytes = if let Some(s) = json_val.get("data").and_then(|v| v.as_str()) {
                s.as_bytes().to_vec()
            } else {
                extract_bytes(json_val.get("data")).unwrap_or_default()
            };
            return Ok(data_bytes);
        }

        // Chunked file: reassemble requested range from covering chunks
        let mut out = vec![0u8; length as usize];
        let chunks_arr = json_val
            .get("chunks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                SurrealFsError::Database("Missing chunks in range read response".to_string())
            })?;

        for c in chunks_arr {
            let c_offset = c.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let c_length = c.get("length").and_then(|v| v.as_u64()).unwrap_or(0);
            let codec = c.get("codec").and_then(|v| v.as_str()).unwrap_or("none");

            let raw_bytes = extract_bytes(c.get("bytes")).unwrap_or_default();
            let decompressed = decompress_chunk(&raw_bytes, codec)
                .map_err(|e| SurrealFsError::Other(format!("Failed to decompress chunk: {}", e)))?;

            let start = std::cmp::max(offset, c_offset);
            let end = std::cmp::min(offset + length, c_offset + c_length);
            if start < end {
                let chunk_slice_start = (start - c_offset) as usize;
                let chunk_slice_len = (end - start) as usize;
                let target_start = (start - offset) as usize;

                if chunk_slice_start + chunk_slice_len <= decompressed.len() {
                    out[target_start..target_start + chunk_slice_len].copy_from_slice(
                        &decompressed[chunk_slice_start..chunk_slice_start + chunk_slice_len],
                    );
                }
            }
        }

        Ok(out)
    }

    /// Return instant disk usage statistics for a path.
    pub async fn du(&self, path: &str) -> Result<UsageStats> {
        let norm = normalize_path(path);
        let sql = "RETURN fn::sfs_du($path, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("caller", self.caller.clone()))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let s_val = val
            .ok_or_else(|| SurrealFsError::NotFound(format!("Path not found for du: {}", path)))?;
        value_to_serde(s_val)
    }

    /// Clean up unreferenced blobs older than max_age_secs.
    pub async fn gc_blobs(&self, max_age_secs: u64) -> Result<usize> {
        let sql = "RETURN fn::sfs_gc_blobs($max_age_secs);";
        let mut res = self
            .db
            .query(sql)
            .bind(("max_age_secs", max_age_secs as i64))
            .await?;
        let count: Option<i64> = res.take(0usize)?;
        Ok(count.unwrap_or(0) as usize)
    }

    /// Return symbols for a file or folder subtree.
    pub async fn symbols(&self, path: &str) -> Result<Vec<CodeSymbol>> {
        let norm = normalize_path(path);
        let sql = "RETURN fn::sfs_symbols($path, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("caller", self.caller.as_deref().unwrap_or("root")))
            .await?;
        let rows: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut out = Vec::new();
        for r in rows {
            out.push(value_to_serde(r)?);
        }
        Ok(out)
    }

    /// Look up symbol definitions by exact or qualified name.
    pub async fn definition(&self, name: &str) -> Result<Vec<CodeSymbol>> {
        let sql = "RETURN fn::sfs_definition($name, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("name", name.to_string()))
            .bind(("caller", self.caller.as_deref().unwrap_or("root")))
            .await?;
        let rows: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut out = Vec::new();
        for r in rows {
            out.push(value_to_serde(r)?);
        }
        Ok(out)
    }

    /// Get hierarchical folder digest for a directory.
    pub async fn digest(&self, path: &str) -> Result<FolderDigest> {
        let norm = normalize_path(path);
        let sql = "RETURN fn::sfs_digest($path, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("caller", self.caller.clone()))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let s_val = val.ok_or_else(|| {
            SurrealFsError::NotFound(format!("Path not found for digest: {}", path))
        })?;
        value_to_serde(s_val)
    }

    /// Context packing tool for questions within a token budget (§20.6).
    pub async fn pack(
        &self,
        question: &str,
        budget: usize,
        scope: Option<&str>,
    ) -> Result<PackResult> {
        let norm_scope = scope.map(normalize_path);
        let sql = "RETURN fn::sfs_pack($question, $budget, $scope, $caller);";
        let mut res = self
            .db
            .query(sql)
            .bind(("question", question.to_string()))
            .bind(("budget", budget as i64))
            .bind(("scope", norm_scope))
            .bind(("caller", self.caller.as_deref().unwrap_or("root")))
            .await?;
        let val: Option<SValue> = res.take(0usize)?;
        let s_val = val.ok_or_else(|| {
            SurrealFsError::Database("Empty response from fn::sfs_pack".to_string())
        })?;
        let json_val: Value = value_to_serde(s_val)?;
        let candidates_arr = json_val
            .get("candidates")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut blocks = Vec::new();
        for c in candidates_arr {
            let path = c
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = c
                .get("snippet")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let score = c.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let tokens = crate::understanding::estimate_tokens(&snippet);
            blocks.push(PackedBlock {
                path,
                line_start: 1,
                line_end: snippet.lines().count().max(1),
                content: snippet,
                tokens,
                score,
            });
        }

        Ok(crate::understanding::pack_blocks(question, budget, blocks))
    }

    /// Extract and index symbols for a specific file.
    pub async fn index_file_symbols(&self, path: &str) -> Result<usize> {
        let content_str = self.read_text(path).await?;
        let detected = crate::understanding::detect_type_and_language(path, content_str.as_bytes());
        let lang = detected.language.unwrap_or_else(|| "text".to_string());
        let symbols = crate::understanding::extract_symbols(path, &content_str, &lang);

        let norm = normalize_path(path);
        let symbols_json =
            serde_json::to_value(&symbols).map_err(|e| SurrealFsError::Other(e.to_string()))?;

        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                DELETE symbol WHERE file_id = $file_id;
                FOR $sym IN $symbols {
                    CREATE symbol CONTENT {
                        file_id: $file_id,
                        name: $sym.name,
                        qualified: $sym.qualified,
                        kind: $sym.kind,
                        language: $sym.language,
                        signature: $sym.signature,
                        doc: $sym.doc,
                        line_start: $sym.line_start,
                        line_end: $sym.line_end
                    };
                };
                RETURN array::len($symbols);
            };
        "#;
        let mut res = self
            .db
            .query(sql)
            .bind(("path", norm))
            .bind(("symbols", symbols_json))
            .await?;
        let count: Option<i64> = res.take(0usize)?;
        Ok(count.unwrap_or(0) as usize)
    }

    /// Claim pipeline jobs for execution by a worker.
    pub async fn claim_jobs(
        &self,
        worker_id: &str,
        kinds: &[&str],
        limit: usize,
    ) -> Result<Vec<PipelineJob>> {
        let kinds_vec: Vec<String> = kinds.iter().map(|s| s.to_string()).collect();
        let sql = "RETURN fn::sfs_job_claim($worker_id, $kinds, $limit);";
        let mut res = self
            .db
            .query(sql)
            .bind(("worker_id", worker_id.to_string()))
            .bind(("kinds", kinds_vec))
            .bind(("limit", limit as i64))
            .await?;
        let rows: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut out = Vec::new();
        for r in rows {
            out.push(value_to_serde(r)?);
        }
        Ok(out)
    }

    /// Complete a pipeline job.
    pub async fn complete_job(&self, job_id: &str, error: Option<&str>) -> Result<()> {
        let sql = "RETURN fn::sfs_job_complete($job_id, $error);";
        self.db
            .query(sql)
            .bind(("job_id", job_id.to_string()))
            .bind(("error", error.map(|s| s.to_string())))
            .await?;
        Ok(())
    }

    /// Retrieve entities mentioned in a file or directory (§20.7).
    pub async fn entities(&self, path: &str) -> Result<Vec<EntityRecord>> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_entities($path, $caller);")
            .bind(("path", norm))
            .bind((
                "caller",
                self.caller.clone().unwrap_or_else(|| "root".to_string()),
            ))
            .await?;
        let items: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut records = Vec::new();
        for item in items {
            records.push(value_to_serde(item)?);
        }
        Ok(records)
    }

    /// Records entities for a file and creates mentions relations (§20.7).
    pub async fn record_entities(&self, path: &str, entities: &[ExtractedEntity]) -> Result<()> {
        let norm = normalize_path(path);
        let entities_json =
            serde_json::to_value(entities).map_err(|e| SurrealFsError::Other(e.to_string()))?;
        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                FOR $ent IN $entities {
                    LET $eid = type::record('entity', [$ent.name, $ent.kind]);
                    UPSERT $eid MERGE { name: $ent.name, kind: $ent.kind, created_at: time::now() };
                    RELATE $file_id->mentions->$eid;
                };
                RETURN true;
            };
        "#;
        self.db
            .query(sql)
            .bind(("path", norm))
            .bind(("entities", entities_json))
            .await?;
        Ok(())
    }

    /// Query tabular file rows (§21.6).
    pub async fn query_table(&self, path: &str, limit: Option<usize>) -> Result<Vec<TableRow>> {
        let norm = normalize_path(path);
        let mut res = self
            .db
            .query("RETURN fn::sfs_query_table($path, $limit, $caller);")
            .bind(("path", norm))
            .bind(("limit", limit.map(|l| l as i64)))
            .bind((
                "caller",
                self.caller.clone().unwrap_or_else(|| "root".to_string()),
            ))
            .await?;
        let items: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut rows = Vec::new();
        for item in items {
            rows.push(value_to_serde(item)?);
        }
        Ok(rows)
    }

    /// Parse CSV/TSV file content and load into file_row table (§21.6).
    pub async fn load_tabular_file(&self, path: &str, max_rows: usize) -> Result<usize> {
        let norm = normalize_path(path);
        let text = self.read_text(&norm).await?;
        let (_headers, rows) = parse_tabular_records(&text, max_rows);
        let count = rows.len();
        let mut indexed_rows = Vec::new();
        for (idx, r) in rows.into_iter().enumerate() {
            indexed_rows.push(serde_json::json!({
                "row_idx": idx as i64,
                "data": r,
            }));
        }
        let rows_json = serde_json::to_value(&indexed_rows)
            .map_err(|e| SurrealFsError::Other(e.to_string()))?;
        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                DELETE file_row WHERE file_id = $file_id;
                FOR $r IN $rows {
                    CREATE file_row CONTENT {
                        file_id: $file_id,
                        row_idx: $r.row_idx,
                        data: $r.data
                    };
                };
                RETURN array::len($rows);
            };
        "#;
        self.db
            .query(sql)
            .bind(("path", norm))
            .bind(("rows", rows_json))
            .await?;
        Ok(count)
    }

    /// Sets the SimHash fingerprint on a file (§20.8).
    pub async fn set_simhash(&self, path: &str, simhash: u64) -> Result<()> {
        let norm = normalize_path(path);
        let sql = r#"
            RETURN {
                LET $file_id = fn::sfs_resolve($path);
                IF $file_id IS NONE { THROW 'sfs:not_found' };
                UPDATE $file_id MERGE { meta: { simhash: $simhash } };
                RETURN true;
            };
        "#;
        self.db
            .query(sql)
            .bind(("path", norm))
            .bind(("simhash", simhash as i64))
            .await?;
        Ok(())
    }

    /// Checks text against existing files using SimHash lexical signatures (§20.8).
    pub async fn check_near_duplicates(
        &self,
        text: &str,
        threshold: f64,
    ) -> Result<Vec<(String, f64)>> {
        let h = compute_simhash(text);
        let mut res = self
            .db
            .query("SELECT path, meta.simhash AS simhash FROM file WHERE meta.simhash IS NOT NONE;")
            .await?;
        let items: Vec<SValue> = res.take(0usize).unwrap_or_default();
        let mut matches = Vec::new();
        for item in items {
            let val: serde_json::Value = value_to_serde(item)?;
            if let (Some(p), Some(num)) = (
                val.get("path").and_then(|v| v.as_str()),
                val.get("simhash").and_then(|v| v.as_i64()),
            ) {
                let target_h = num as u64;
                let sim = simhash_similarity(h, target_h);
                if sim >= threshold {
                    matches.push((p.to_string(), sim));
                }
            }
        }
        matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(matches)
    }

    /// Records a Git commit mapped to a SurrealFS tree (§21.5).
    pub async fn record_git_commit(
        &self,
        commit: &crate::git::GitCommit,
        target_root: &str,
    ) -> Result<()> {
        let sql = r#"
            CREATE git_commit CONTENT {
                sha: $sha,
                author_name: $author_name,
                author_email: $author_email,
                timestamp: $timestamp,
                message: $message,
                target_root: $target_root,
                created_at: time::now()
            };
        "#;
        self.db
            .query(sql)
            .bind(("sha", commit.sha.as_str()))
            .bind(("author_name", commit.author_name.as_str()))
            .bind(("author_email", commit.author_email.as_str()))
            .bind(("timestamp", commit.timestamp.as_str()))
            .bind(("message", commit.message.as_str()))
            .bind(("target_root", target_root))
            .await?;
        Ok(())
    }

    /// Imports a Git repository into SurrealFS, respecting .gitignore and mapping commits (§21.5).
    pub async fn import_git(
        &self,
        opts: &crate::git::GitImportOptions,
    ) -> Result<crate::git::GitImportResult> {
        crate::git::import_git_repository(self, opts).await
    }
}
