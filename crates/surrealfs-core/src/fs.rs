//! Core async SurrealFs implementation for Rust.

use crate::crdt::CrdtDoc;
use crate::errors::{Result, SurrealFsError};
use crate::models::{
    FileEntry, FileLock, FileVersion, GrepMatch, MailboxMessage, SearchHit, SectionHit,
    WorkspaceDiff,
};
use crate::paths::normalize_path;
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
}
