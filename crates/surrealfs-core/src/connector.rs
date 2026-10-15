//! External Source Mirrors (§23.1).
//! Mirrors external sources under `/sources/<connector>/...` as Markdown and CSV with source metadata.

use crate::errors::Result;
use crate::fs::SurrealFs;
use crate::paths::normalize_path;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Report returned after syncing an external source mirror.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncReport {
    pub connector: String,
    pub files_synced: usize,
    pub bytes_synced: u64,
    pub errors: Vec<String>,
}

/// Generic item representing a document or resource from an external service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDocument {
    pub id: String,
    pub path: String,
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub source_url: String,
    pub source_version: Option<String>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub mode: Option<u32>,
}

/// Connector interface for external services.
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait Connector: Send + Sync {
    /// Unique identifier for connector (e.g. "gdrive", "notion", "slack", "github").
    fn name(&self) -> &str;

    /// Fetches and returns source documents to be synchronized.
    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>>;

    /// Synchronizes external documents into SurrealFS under `/sources/<name>/...`.
    async fn sync(&self, fs: &SurrealFs) -> Result<SyncReport> {
        let base_prefix = format!("/sources/{}", self.name());
        fs.mkdir(&base_prefix, true).await?;

        let docs = self.fetch_documents().await?;
        let mut report = SyncReport {
            connector: self.name().to_string(),
            ..Default::default()
        };

        for doc in docs {
            let full_path = normalize_path(&format!(
                "{}/{}",
                base_prefix,
                doc.path.trim_start_matches('/')
            ));

            // Ensure parent directory exists
            if let Some(parent) = full_path.rsplit_once('/').map(|(p, _)| p) {
                if !parent.is_empty() && parent != "/" {
                    let _ = fs.mkdir(parent, true).await;
                }
            }

            let bytes = doc.content.as_bytes();
            let len = bytes.len() as u64;

            // Write content
            if let Err(e) = fs.write_bytes(&full_path, bytes, None).await {
                report
                    .errors
                    .push(format!("Failed to write {}: {}", full_path, e));
                continue;
            }

            // Set metadata attributes
            let meta_sql = r#"
                UPDATE fn::sfs_resolve($path) SET
                    meta.source = $source,
                    meta.source_id = $source_id,
                    meta.source_url = $source_url,
                    meta.source_version = $source_version,
                    meta.synced_at = time::now(),
                    meta.is_mirror = true;
            "#;
            let _ = fs
                .db
                .query(meta_sql)
                .bind(("path", full_path.clone()))
                .bind(("source", self.name().to_string()))
                .bind(("source_id", doc.id.clone()))
                .bind(("source_url", doc.source_url.clone()))
                .bind(("source_version", doc.source_version.clone()))
                .await;

            // Apply mapped permissions if specified
            if let Some(grp) = doc.group {
                let _ = fs.chgrp(&full_path, Some(&grp)).await;
            }
            if let Some(m) = doc.mode {
                let _ = fs.chmod(&full_path, m).await;
            }

            report.files_synced += 1;
            report.bytes_synced += len;
        }

        Ok(report)
    }
}

/// Helper to fetch documents over HTTP from a provider or mock API endpoint.
async fn fetch_remote_documents(
    api_url: &str,
    bearer_token: Option<&str>,
) -> Result<Vec<SourceDocument>> {
    let client = reqwest::Client::new();
    let mut req = client.get(api_url);
    if let Some(tok) = bearer_token {
        req = req.bearer_auth(tok);
    }
    let resp = req.send().await.map_err(|e| {
        crate::errors::SurrealFsError::Other(format!("Connector HTTP request failed: {}", e))
    })?;
    if !resp.status().is_success() {
        return Err(crate::errors::SurrealFsError::Other(format!(
            "Connector HTTP status {}",
            resp.status()
        )));
    }
    let val: serde_json::Value = resp.json().await.map_err(|e| {
        crate::errors::SurrealFsError::Other(format!("Connector JSON parse failed: {}", e))
    })?;
    if let Ok(docs) = serde_json::from_value::<Vec<SourceDocument>>(val.clone()) {
        return Ok(docs);
    }
    if let Some(arr) = val
        .get("files")
        .or_else(|| val.get("results"))
        .or_else(|| val.get("items"))
        .or_else(|| val.get("messages"))
        .and_then(|v| v.as_array())
    {
        let mut out = Vec::new();
        for item in arr {
            if let Ok(doc) = serde_json::from_value::<SourceDocument>(item.clone()) {
                out.push(doc);
            } else {
                let id = item
                    .get("id")
                    .or_else(|| item.get("ts"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("doc")
                    .to_string();
                let title = item
                    .get("name")
                    .or_else(|| item.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let path = item
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{}.md", title.replace(' ', "_")));
                let content = item
                    .get("content")
                    .or_else(|| item.get("text"))
                    .or_else(|| item.get("body"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content_type = item
                    .get("mimeType")
                    .or_else(|| item.get("content_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("text/markdown")
                    .to_string();
                let source_url = item
                    .get("url")
                    .or_else(|| item.get("html_url"))
                    .or_else(|| item.get("webViewLink"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(api_url)
                    .to_string();
                out.push(SourceDocument {
                    id,
                    path,
                    title,
                    content,
                    content_type,
                    source_url,
                    source_version: item
                        .get("version")
                        .or_else(|| item.get("sha"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    owner: None,
                    group: None,
                    mode: None,
                });
            }
        }
        return Ok(out);
    }
    Ok(Vec::new())
}

/// Connector for Google Drive (supports HTTP API endpoint or in-memory fixture documents).
pub struct GoogleDriveConnector {
    pub docs: Vec<SourceDocument>,
    pub api_url: Option<String>,
    pub bearer_token: Option<String>,
}

impl GoogleDriveConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self {
            docs,
            api_url: None,
            bearer_token: None,
        }
    }

    pub fn with_http(api_url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            docs: Vec::new(),
            api_url: Some(api_url.into()),
            bearer_token,
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Connector for GoogleDriveConnector {
    fn name(&self) -> &str {
        "gdrive"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        if let Some(ref url) = self.api_url {
            return fetch_remote_documents(url, self.bearer_token.as_deref()).await;
        }
        Ok(self.docs.clone())
    }
}

/// Connector for Notion (supports HTTP API endpoint or in-memory fixture documents).
pub struct NotionConnector {
    pub docs: Vec<SourceDocument>,
    pub api_url: Option<String>,
    pub bearer_token: Option<String>,
}

impl NotionConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self {
            docs,
            api_url: None,
            bearer_token: None,
        }
    }

    pub fn with_http(api_url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            docs: Vec::new(),
            api_url: Some(api_url.into()),
            bearer_token,
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Connector for NotionConnector {
    fn name(&self) -> &str {
        "notion"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        if let Some(ref url) = self.api_url {
            return fetch_remote_documents(url, self.bearer_token.as_deref()).await;
        }
        Ok(self.docs.clone())
    }
}

/// Connector for GitHub (supports HTTP API endpoint or in-memory fixture documents).
pub struct GitHubConnector {
    pub docs: Vec<SourceDocument>,
    pub api_url: Option<String>,
    pub bearer_token: Option<String>,
}

impl GitHubConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self {
            docs,
            api_url: None,
            bearer_token: None,
        }
    }

    pub fn with_http(api_url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            docs: Vec::new(),
            api_url: Some(api_url.into()),
            bearer_token,
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Connector for GitHubConnector {
    fn name(&self) -> &str {
        "github"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        if let Some(ref url) = self.api_url {
            return fetch_remote_documents(url, self.bearer_token.as_deref()).await;
        }
        Ok(self.docs.clone())
    }
}

/// Connector for Slack (supports HTTP API endpoint or in-memory fixture documents).
pub struct SlackConnector {
    pub docs: Vec<SourceDocument>,
    pub api_url: Option<String>,
    pub bearer_token: Option<String>,
}

impl SlackConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self {
            docs,
            api_url: None,
            bearer_token: None,
        }
    }

    pub fn with_http(api_url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            docs: Vec::new(),
            api_url: Some(api_url.into()),
            bearer_token,
        }
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl Connector for SlackConnector {
    fn name(&self) -> &str {
        "slack"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        if let Some(ref url) = self.api_url {
            return fetch_remote_documents(url, self.bearer_token.as_deref()).await;
        }
        Ok(self.docs.clone())
    }
}

/// Central manager orchestrating all registered external connectors.
#[derive(Default)]
pub struct ConnectorManager {
    connectors: Vec<Box<dyn Connector>>,
}

impl ConnectorManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<C: Connector + 'static>(&mut self, connector: C) {
        self.connectors.push(Box::new(connector));
    }

    /// Run sync across all registered connectors.
    pub async fn sync_all(&self, fs: &SurrealFs) -> Result<Vec<SyncReport>> {
        let mut reports = Vec::new();
        for conn in &self.connectors {
            let rep = conn.sync(fs).await?;
            reports.push(rep);
        }
        Ok(reports)
    }
}
