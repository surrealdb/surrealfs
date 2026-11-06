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
            .map_err(|e| {
                SurrealFsError::InvalidPath(format!("Invalid base64 in auth header: {}", e))
            })?;
        let auth_str = String::from_utf8(decoded).map_err(|e| {
            SurrealFsError::InvalidPath(format!("Invalid UTF-8 in credentials: {}", e))
        })?;

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
            // Parse Credential=<access_key>/<date>/<region>/<service>/aws4_request and Signature=<hex>
            let cred_val = trimmed.split(',').find_map(|part| {
                let p = part.trim();
                p.strip_prefix("Credential=")
                    .or_else(|| p.strip_prefix("AWS4-HMAC-SHA256 Credential="))
                    .map(|v| v.trim())
            });

            let sig_val = trimmed
                .split(',')
                .find_map(|part| part.trim().strip_prefix("Signature=").map(|v| v.trim()));

            let (Some(cred_str), Some(signature)) = (cred_val, sig_val) else {
                return Err(SurrealFsError::PermissionDenied(
                    "Malformed AWS4-HMAC-SHA256 header: missing Credential or Signature"
                        .to_string(),
                ));
            };

            if signature.is_empty() {
                return Err(SurrealFsError::PermissionDenied(
                    "Empty S3 Signature".to_string(),
                ));
            }

            let mut cred_parts = cred_str.split('/');
            let access_key = cred_parts.next().unwrap_or("");
            let date = cred_parts.next().unwrap_or("19700101");
            let region = cred_parts.next().unwrap_or("us-east-1");
            let service = cred_parts.next().unwrap_or("s3");

            if !access_key.is_empty() {
                if let Some(cred) = self.fs.resolve_credential("s3", access_key).await? {
                    if !cred.enabled {
                        return Err(SurrealFsError::PermissionDenied(
                            "S3 credential disabled".to_string(),
                        ));
                    }
                    let Some(expected_secret) = cred.secret_hash.as_deref() else {
                        return Err(SurrealFsError::PermissionDenied(
                            "S3 credential has no secret configured".to_string(),
                        ));
                    };

                    if self.verify_s3_signature(signature, expected_secret, date, region, service) {
                        return Ok(AuthenticatedUser {
                            user_id: cred.user_id,
                            kind: cred.kind,
                            identifier: cred.identifier,
                        });
                    } else {
                        return Err(SurrealFsError::PermissionDenied(
                            "S3 signature verification failed".to_string(),
                        ));
                    }
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

    /// Computes a deterministic SigV4 signature token for `(secret, date, region, service)`.
    pub fn compute_s3_signature(secret: &str, date: &str, region: &str, service: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;

        let hmac_sign = |key: &[u8], data: &[u8]| -> Vec<u8> {
            let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        };

        let k_secret = format!("AWS4{}", secret);
        let k_date = hmac_sign(k_secret.as_bytes(), date.as_bytes());
        let k_region = hmac_sign(&k_date, region.as_bytes());
        let k_service = hmac_sign(&k_region, service.as_bytes());
        let k_signing = hmac_sign(&k_service, b"aws4_request");
        let sig = hmac_sign(&k_signing, b"surrealfs-s3-request");
        hex::encode(sig)
    }

    fn verify_s3_signature(
        &self,
        signature: &str,
        expected_hash_or_secret: &str,
        date: &str,
        region: &str,
        service: &str,
    ) -> bool {
        if self.verify_secret(signature, Some(expected_hash_or_secret)) {
            return true;
        }
        let derived = Self::compute_s3_signature(expected_hash_or_secret, date, region, service);
        constant_time_eq(signature.as_bytes(), derived.as_bytes())
    }

    fn verify_secret(&self, provided: &str, expected_hash_or_secret: Option<&str>) -> bool {
        match expected_hash_or_secret {
            Some(expected) if !expected.is_empty() => {
                if expected.starts_with(PBKDF2_PREFIX) {
                    return verify_pbkdf2(provided, expected);
                }
                // Stored values without a scheme prefix are a plaintext secret
                // (required for S3 signing keys) or an unsalted SHA-256 hex digest.
                if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
                    return true;
                }
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(provided.as_bytes());
                let hex_hash = hex::encode(hasher.finalize());
                constant_time_eq(hex_hash.as_bytes(), expected.as_bytes())
            }
            _ => false, // Reject credentials without a secret
        }
    }
}

const PBKDF2_PREFIX: &str = "pbkdf2-sha256$";
const PBKDF2_ITERATIONS: u32 = 210_000;
const PBKDF2_MAX_ITERATIONS: u32 = 10_000_000;

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let prf = |data: &[&[u8]]| -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(password).expect("HMAC accepts any key length");
        for d in data {
            mac.update(d);
        }
        mac.finalize().into_bytes().into()
    };
    let mut u = prf(&[salt, &1u32.to_be_bytes()]);
    let mut out = u;
    for _ in 1..iterations {
        u = prf(&[&u]);
        for (o, b) in out.iter_mut().zip(u.iter()) {
            *o ^= b;
        }
    }
    out
}

/// Hashes a credential secret as `pbkdf2-sha256$<iterations>$<salt>$<hash>` with a random
/// salt. Not suitable for S3 credentials, whose signing key must be recoverable.
pub fn hash_secret(secret: &str) -> Result<String> {
    use base64::Engine;
    let mut salt = [0u8; 16];
    getrandom_fill(&mut salt)?;
    let hash = pbkdf2_sha256(secret.as_bytes(), &salt, PBKDF2_ITERATIONS);
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD;
    Ok(format!(
        "{PBKDF2_PREFIX}{PBKDF2_ITERATIONS}${}${}",
        b64.encode(salt),
        b64.encode(hash)
    ))
}

#[cfg(all(unix, not(target_arch = "wasm32")))]
fn getrandom_fill(buf: &mut [u8]) -> Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .map_err(|e| SurrealFsError::PermissionDenied(format!("no OS randomness available: {e}")))
}

#[cfg(not(all(unix, not(target_arch = "wasm32"))))]
fn getrandom_fill(_buf: &mut [u8]) -> Result<()> {
    Err(SurrealFsError::PermissionDenied(
        "secret hashing needs an OS randomness source, unavailable on this platform".to_string(),
    ))
}

fn verify_pbkdf2(provided: &str, stored: &str) -> bool {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD;
    let mut parts = stored[PBKDF2_PREFIX.len()..].split('$');
    let (Some(iters), Some(salt), Some(hash), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(iters) = iters.parse::<u32>() else {
        return false;
    };
    if iters == 0 || iters > PBKDF2_MAX_ITERATIONS {
        return false;
    }
    let (Ok(salt), Ok(hash)) = (b64.decode(salt), b64.decode(hash)) else {
        return false;
    };
    constant_time_eq(&pbkdf2_sha256(provided.as_bytes(), &salt, iters), &hash)
}

/// Constant-time byte slice equality check to prevent timing side-channels.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_rfc_vector() {
        // RFC 7914 §11: PBKDF2-HMAC-SHA-256, P="passwd", S="salt", c=1, dkLen=32.
        let out = pbkdf2_sha256(b"passwd", b"salt", 1);
        assert_eq!(
            hex::encode(out),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
        );
    }

    #[test]
    fn hashed_secret_round_trips_and_is_salted() {
        let a = hash_secret("hunter2").unwrap();
        let b = hash_secret("hunter2").unwrap();
        assert_ne!(a, b);
        assert!(verify_pbkdf2("hunter2", &a));
        assert!(!verify_pbkdf2("hunter3", &a));
        assert!(!verify_pbkdf2("hunter2", "pbkdf2-sha256$999999999$AA$AA"));
    }

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
