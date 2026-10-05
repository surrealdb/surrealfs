use thiserror::Error;

#[derive(Error, Debug)]
pub enum SurrealFsError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Directory not empty: {0}")]
    DirectoryNotEmpty(String),

    #[error("Is a directory: {0}")]
    IsADirectory(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("IO error: {0}")]
    Io(String),

    #[error("Lock error: {0}")]
    Lock(String),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SurrealFsError>;

impl From<surrealdb::Error> for SurrealFsError {
    fn from(err: surrealdb::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("sfs:not_found") || msg.contains("not found") {
            SurrealFsError::NotFound(msg)
        } else if msg.contains("sfs:permission")
            || msg.contains("denied")
            || msg.contains("Permissions")
        {
            SurrealFsError::PermissionDenied(msg)
        } else if msg.contains("sfs:conflict")
            || msg.contains("conflict")
            || msg.contains("Transaction")
        {
            SurrealFsError::Conflict(msg)
        } else if msg.contains("sfs:is_a_directory") {
            SurrealFsError::IsADirectory(msg)
        } else if msg.contains("sfs:not_empty") {
            SurrealFsError::DirectoryNotEmpty(msg)
        } else if msg.contains("sfs:already_exists") {
            SurrealFsError::AlreadyExists(msg)
        } else {
            SurrealFsError::Database(msg)
        }
    }
}

impl From<anyhow::Error> for SurrealFsError {
    fn from(err: anyhow::Error) -> Self {
        SurrealFsError::Other(err.to_string())
    }
}
