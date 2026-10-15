use reqwest::Client;
use serde::{Deserialize, Serialize};
use surrealfs_core::understanding::{compute_simhash, extract_entities, split_markdown_sections};
use surrealfs_core::{Result, SurrealFs, SurrealFsError};

pub const DEFAULT_EMBED_MODEL: &str = "text-embedding-3-small";
pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const MAX_EMBED_CHARS: usize = 24_000;

#[derive(Clone, Debug)]
pub struct Embedder {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingItem>,
}

#[derive(Deserialize)]
struct EmbeddingItem {
    embedding: Vec<f32>,
}

impl Embedder {
    pub fn new(
        api_key: impl Into<String>,
        base_url: Option<String>,
        model: Option<String>,
    ) -> Self {
        let base = base_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string())
            .trim_end_matches('/')
            .to_string();
        let model = model
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string());
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            base_url: base,
            model,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())?;
        let base_url = std::env::var("OPENAI_BASE_URL").ok();
        let model = std::env::var("SURREALFS_EMBED_MODEL").ok();
        Some(Self::new(key, base_url, model))
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let truncated: String = text.chars().take(MAX_EMBED_CHARS).collect();
        if truncated.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut batch = self.embed_batch(&[truncated]).await?;
        batch
            .pop()
            .ok_or_else(|| SurrealFsError::Other("Empty embedding response".to_string()))
    }

    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inputs: Vec<String> = texts
            .iter()
            .map(|t| t.chars().take(MAX_EMBED_CHARS).collect())
            .collect();
        let url = format!("{}/embeddings", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&EmbeddingRequest {
                model: &self.model,
                input: &inputs,
            })
            .send()
            .await
            .map_err(|e| SurrealFsError::Other(format!("Embedding HTTP error: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SurrealFsError::Other(format!(
                "Embedding API returned {}: {}",
                status, body
            )));
        }

        let parsed: EmbeddingResponse = resp
            .json()
            .await
            .map_err(|e| SurrealFsError::Other(format!("Embedding JSON parse error: {}", e)))?;
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }
}

/// Automatically indexes a file after a write/edit/append on the server:
/// - Computes & stores SimHash fingerprint
/// - Extracts & indexes AST/regex code symbols
/// - Extracts & records Markdown/text entities
/// - Splits Markdown sections & generates OpenAI vector embeddings (if `embedder` is configured)
pub async fn index_file_on_write(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    path: &str,
) -> Result<()> {
    if path.starts_with("/_sessions/") || path == "/_sessions" {
        return Ok(());
    }
    let content = match fs.read_text(path).await {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    if content.trim().is_empty() {
        return Ok(());
    }

    // 1. SimHash lexical signature
    let simhash = compute_simhash(&content);
    let _ = fs.set_simhash(path, simhash).await;

    // 2. Code symbol indexing
    let _ = fs.index_file_symbols(path).await;

    // 3. Entity extraction
    let entities = extract_entities(&content);
    if !entities.is_empty() {
        let _ = fs.record_entities(path, &entities).await;
    }

    // 4. Markdown section splitting + vector embeddings
    let sections = split_markdown_sections(&content);
    if let Some(emb) = embedder {
        let doc_text = format!("{}\n\n{}", path, content);
        if let Ok(vec) = emb.embed(&doc_text).await {
            if !vec.is_empty() {
                let _ = fs.set_embedding(path, &vec, emb.model()).await;
            }
        }

        if !sections.is_empty() {
            let sec_inputs: Vec<String> = sections
                .iter()
                .map(|s| format!("{}#{}:\n{}", path, s.heading, s.content))
                .collect();
            let sec_vecs = emb.embed_batch(&sec_inputs).await.unwrap_or_default();
            let paired: Vec<_> = sections
                .into_iter()
                .enumerate()
                .map(|(i, sec)| (sec, sec_vecs.get(i).cloned()))
                .collect();
            let _ = fs.index_file_sections(path, &paired).await;
        }
    } else if !sections.is_empty() {
        let paired: Vec<_> = sections.into_iter().map(|sec| (sec, None)).collect();
        let _ = fs.index_file_sections(path, &paired).await;
    }

    Ok(())
}

/// Drains pending `pipeline_job` rows from SurrealDB and runs server-side indexing.
pub async fn process_pending_jobs(
    fs: &SurrealFs,
    embedder: Option<&Embedder>,
    worker_id: &str,
    limit: usize,
) -> Result<usize> {
    let jobs = fs
        .claim_jobs(
            worker_id,
            &["embed", "symbols", "entities", "sections"],
            limit,
        )
        .await?;
    let mut completed = 0;
    for job in jobs {
        let res = if let Some(ref p) = job.path {
            index_file_on_write(fs, embedder, p).await
        } else {
            Ok(())
        };
        let err_msg = res.err().map(|e| e.to_string());
        let _ = fs.complete_job(&job.id, err_msg.as_deref()).await;
        if err_msg.is_none() {
            completed += 1;
        }
    }
    Ok(completed)
}
