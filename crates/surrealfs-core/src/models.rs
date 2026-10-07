use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default, deserialize_with = "deserialize_null_as_false")]
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
    #[serde(default, deserialize_with = "deserialize_null_as_false")]
    pub crdt: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

fn deserialize_null_as_false<'de, D>(deserializer: D) -> std::result::Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<bool>::deserialize(deserializer)?;
    Ok(opt.unwrap_or(false))
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
pub struct SectionHit {
    pub path: String,
    pub heading: String,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub score: f64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobChunk {
    pub chunk_id: String,
    pub offset: u64,
    pub length: u64,
    pub uncompressed_size: u64,
    pub stored_size: u64,
    pub codec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadSession {
    pub upload_id: String,
    pub missing_chunks: Vec<String>,
    pub received_chunks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    pub path: String,
    pub files: u64,
    pub logical_bytes: u64,
    pub stored_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSymbol {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    pub name: String,
    pub qualified: String,
    pub kind: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestNotableFile {
    pub filename: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderDigest {
    pub path: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub notable_files: Vec<DigestNotableFile>,
    #[serde(default)]
    pub subfolders: Vec<String>,
    #[serde(default)]
    pub total_readable: usize,
    #[serde(default)]
    pub private_unsummarised: usize,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedBlock {
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub tokens: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackResult {
    pub question: String,
    pub budget: usize,
    pub used_tokens: usize,
    pub blocks: Vec<PackedBlock>,
    pub formatted: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineJob {
    pub id: String,
    pub file_id: String,
    #[serde(default)]
    pub path: Option<String>,
    pub kind: String,
    pub source_hash: String,
    #[serde(default)]
    pub status: String,
    pub attempts: i64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub row_idx: i64,
    #[serde(default)]
    pub sheet: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CredentialRecord {
    pub kind: String,
    pub identifier: String,
    #[serde(default)]
    pub secret_hash: Option<String>,
    pub user_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub meta: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}
