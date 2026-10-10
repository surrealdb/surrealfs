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
#[async_trait]
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

/// In-memory mock/configured connector for Google Drive.
pub struct GoogleDriveConnector {
    pub docs: Vec<SourceDocument>,
}

impl GoogleDriveConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl Connector for GoogleDriveConnector {
    fn name(&self) -> &str {
        "gdrive"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        Ok(self.docs.clone())
    }
}

/// In-memory mock/configured connector for Notion.
pub struct NotionConnector {
    pub docs: Vec<SourceDocument>,
}

impl NotionConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl Connector for NotionConnector {
    fn name(&self) -> &str {
        "notion"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        Ok(self.docs.clone())
    }
}

/// In-memory mock/configured connector for GitHub.
pub struct GitHubConnector {
    pub docs: Vec<SourceDocument>,
}

impl GitHubConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl Connector for GitHubConnector {
    fn name(&self) -> &str {
        "github"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
        Ok(self.docs.clone())
    }
}

/// In-memory mock/configured connector for Slack.
pub struct SlackConnector {
    pub docs: Vec<SourceDocument>,
}

impl SlackConnector {
    pub fn new(docs: Vec<SourceDocument>) -> Self {
        Self { docs }
    }
}

#[async_trait]
impl Connector for SlackConnector {
    fn name(&self) -> &str {
        "slack"
    }

    async fn fetch_documents(&self) -> Result<Vec<SourceDocument>> {
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
