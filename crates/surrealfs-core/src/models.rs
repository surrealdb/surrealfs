use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub is_folder: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default = "default_content_type")]
    pub content_type: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: u32,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default = "default_generation")]
    pub generation: u64,
    #[serde(default)]
    pub parent_key: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

fn default_content_type() -> String {
    "application/octet-stream".to_string()
}

fn default_mode() -> u32 {
    0o644
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_generation() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVersion {
    pub path: String,
    pub generation: u64,
    pub author: String,
    #[serde(default = "default_op")]
    pub op: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

fn default_op() -> String {
    "write".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLock {
    pub path: String,
    pub holder: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub acquired_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub entry: FileEntry,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    #[serde(default)]
    pub id: String,
    pub agent_id: String,
    pub op: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub priority: i64,
    #[serde(default = "default_state")]
    pub state: String,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

fn default_state() -> String {
    "pending".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiff {
    pub path: String,
    #[serde(default)]
    pub modified: bool,
    #[serde(default)]
    pub conflict: bool,
    #[serde(default)]
    pub base_gen: Option<u64>,
    #[serde(default)]
    pub branch_gen: Option<u64>,
}
