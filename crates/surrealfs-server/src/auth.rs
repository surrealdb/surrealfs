//! External credential authentication and principal resolution (§22).

use surrealfs_core::errors::{Result, SurrealFsError};
use surrealfs_core::fs::SurrealFs;

/// Validated identity from an external protocol request (§22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub kind: String,
    pub identifier: String,
}

/// Helper for authenticating external protocol clients against the SurrealFS credential store (§22).
#[derive(Clone)]
pub struct Authenticator {
    fs: SurrealFs,
}

impl Authenticator {
    pub fn new(fs: SurrealFs) -> Self {
        Self { fs }
    }

    /// Authenticates HTTP Basic Auth credentials (used by WebDAV and HTTP S3 endpoints).
    pub async fn authenticate_basic(&self, auth_header: &str) -> Result<AuthenticatedUser> {
        let trimmed = auth_header.trim();
        let payload = if let Some(basic) = trimmed.strip_prefix("Basic ") {
            basic.trim()
        } else {
            return Err(SurrealFsError::PermissionDenied(
                "Missing or invalid Basic auth header".to_string(),
            ));
        };

        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|e| SurrealFsError::InvalidPath(format!("Invalid base64 in auth header: {}", e)))?;
        let auth_str = String::from_utf8(decoded)
            .map_err(|e| SurrealFsError::InvalidPath(format!("Invalid UTF-8 in credentials: {}", e)))?;

        let mut parts = auth_str.splitn(2, ':');
        let username = parts.next().unwrap_or("");
        let password = parts.next().unwrap_or("");

        // 1. Try webdav credential
        if let Some(cred) = self.fs.resolve_credential("webdav", username).await? {
            if self.verify_secret(password, cred.secret_hash.as_deref()) {
                return Ok(AuthenticatedUser {
                    user_id: cred.user_id,
                    kind: cred.kind,
                    identifier: cred.identifier,
                });
            }
        }

        // 2. Try s3 credential
        if let Some(cred) = self.fs.resolve_credential("s3", username).await? {
            if self.verify_secret(password, cred.secret_hash.as_deref()) {
                return Ok(AuthenticatedUser {
                    user_id: cred.user_id,
                    kind: cred.kind,
                    identifier: cred.identifier,
                });
            }
        }

        // 3. Fallback to root or default if configured or unauthenticated
        Err(SurrealFsError::PermissionDenied(format!(
            "Authentication failed for user: {}",
            username
        )))
    }

    /// Authenticates a Bearer token.
    pub async fn authenticate_bearer(&self, auth_header: &str) -> Result<AuthenticatedUser> {
        let trimmed = auth_header.trim();
        let token = if let Some(b) = trimmed.strip_prefix("Bearer ") {
            b.trim()
        } else {
            return Err(SurrealFsError::PermissionDenied(
                "Missing Bearer auth header".to_string(),
            ));
        };

        if let Some(cred) = self.fs.resolve_credential("bearer", token).await? {
            return Ok(AuthenticatedUser {
                user_id: cred.user_id,
                kind: cred.kind,
                identifier: cred.identifier,
            });
        }

        Err(SurrealFsError::PermissionDenied(
            "Invalid or expired bearer token".to_string(),
        ))
    }

    /// Authenticates S3 credentials either via AWS4 SigV4 or AccessKey lookup (§22.3).
    pub async fn authenticate_s3(&self, auth_header: Option<&str>) -> Result<AuthenticatedUser> {
        let Some(header) = auth_header else {
            // Check for anonymous access if allowed, otherwise require auth
            return Err(SurrealFsError::PermissionDenied(
                "S3 request requires Authorization header".to_string(),
            ));
        };

        let trimmed = header.trim();
        if trimmed.starts_with("AWS4-HMAC-SHA256") {
            // Parse Credential=<access_key>/<date>/<region>/s3/aws4_request
            let key_opt = trimmed
                .split(',')
                .find(|part| part.trim().starts_with("Credential="))
                .and_then(|cred_part| {
                    let val = cred_part.trim().strip_prefix("Credential=")?.trim();
                    val.split('/').next()
                });

            if let Some(access_key) = key_opt {
                if let Some(cred) = self.fs.resolve_credential("s3", access_key).await? {
                    return Ok(AuthenticatedUser {
                        user_id: cred.user_id,
                        kind: cred.kind,
                        identifier: cred.identifier,
                    });
                }
            }
        } else if trimmed.starts_with("Basic ") {
            return self.authenticate_basic(trimmed).await;
        } else if trimmed.starts_with("Bearer ") {
            return self.authenticate_bearer(trimmed).await;
        }

        Err(SurrealFsError::PermissionDenied(
            "Invalid S3 Authorization header".to_string(),
        ))
    }

    fn verify_secret(&self, provided: &str, expected_hash_or_secret: Option<&str>) -> bool {
        match expected_hash_or_secret {
            Some(expected) => {
                // Support plaintext match or sha256 hex match
                if provided == expected {
                    return true;
                }
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(provided.as_bytes());
                let hex_hash = hex::encode(hasher.finalize());
                hex_hash == expected
            }
            None => true, // No password required if None
        }
    }
}
