//! End-to-end Client-Side Encryption Envelope (§24.3).

use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use crate::paths::normalize_path;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::json;

const ENVELOPE_MAGIC: &[u8; 8] = b"SFSENC01";

/// Encrypt raw data using AES-256-GCM without additional authenticated data.
/// Returns `(ciphertext, nonce_bytes)`.
pub fn encrypt_bytes(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12])> {
    encrypt_bytes_with_aad(key, plaintext, b"")
}

/// Encrypt raw data using AES-256-GCM bound to `aad`.
/// Returns `(ciphertext, nonce_bytes)`.
pub fn encrypt_bytes_with_aad(
    key: &[u8; 32],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<(Vec<u8>, [u8; 12])> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| SurrealFsError::Other(format!("RNG error: {}", e)))?;
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| SurrealFsError::Storage(format!("Encryption failed: {}", e)))?;

    Ok((ciphertext, nonce_bytes))
}

/// Decrypt AES-256-GCM ciphertext using the given key and nonce.
pub fn decrypt_bytes(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8; 12]) -> Result<Vec<u8>> {
    decrypt_bytes_with_aad(key, ciphertext, nonce, b"")
}

/// Decrypt AES-256-GCM ciphertext using the given key, nonce, and `aad`.
pub fn decrypt_bytes_with_aad(
    key: &[u8; 32],
    ciphertext: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from(*nonce);

    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|e| {
            SurrealFsError::PermissionDenied(format!("Decryption failed or invalid key: {}", e))
        })?;

    Ok(plaintext)
}

/// Extension methods on SurrealFs for encrypted file operations.
impl SurrealFs {
    /// Write an encrypted file into SurrealFS with metadata `encrypted = true`.
    /// Stores a self-contained `SFSENC01 || nonce (12B) || ciphertext` envelope bound to the
    /// normalized path as AAD so historical `file_version` snapshots remain decryptable.
    pub async fn write_encrypted(
        &self,
        path: &str,
        data: &[u8],
        key: &[u8; 32],
        key_id: Option<&str>,
    ) -> Result<()> {
        let norm_path = normalize_path(path);
        let (ciphertext, nonce) = encrypt_bytes_with_aad(key, data, norm_path.as_bytes())?;
        let nonce_hex = hex::encode(nonce);

        let mut envelope = Vec::with_capacity(ENVELOPE_MAGIC.len() + 12 + ciphertext.len());
        envelope.extend_from_slice(ENVELOPE_MAGIC);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);

        // Write self-contained envelope bytes
        self.write_bytes(&norm_path, &envelope, None).await?;

        // Update metadata to record encryption parameters
        let sql = "UPDATE fn::sfs_resolve($path) SET meta.encrypted = true, meta.encryption_algo = 'aes-256-gcm', meta.nonce = $nonce, meta.key_id = $key_id;";
        self.db
            .query(sql)
            .bind(("path", norm_path))
            .bind(("nonce", nonce_hex))
            .bind(("key_id", key_id.map(|s| s.to_string())))
            .await?;

        Ok(())
    }

    /// Read and decrypt an encrypted file from SurrealFS.
    pub async fn read_encrypted(&self, path: &str, key: &[u8; 32]) -> Result<Vec<u8>> {
        let norm_path = normalize_path(path);
        let entry = self
            .stat(&norm_path)
            .await?
            .ok_or_else(|| SurrealFsError::NotFound(format!("File {} not found", norm_path)))?;
        let meta = entry.meta.unwrap_or(json!({}));

        let raw_bytes = self.read_bytes(&norm_path).await?;

        // Check for self-contained SFSENC01 envelope first
        if raw_bytes.len() >= 20 && &raw_bytes[..8] == ENVELOPE_MAGIC {
            let mut nonce_bytes = [0u8; 12];
            nonce_bytes.copy_from_slice(&raw_bytes[8..20]);
            let ciphertext = &raw_bytes[20..];
            return decrypt_bytes_with_aad(key, ciphertext, &nonce_bytes, norm_path.as_bytes());
        }

        let is_encrypted = meta
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !is_encrypted {
            return Ok(raw_bytes);
        }

        let nonce_hex = meta.get("nonce").and_then(|v| v.as_str()).ok_or_else(|| {
            SurrealFsError::Storage("Missing encryption nonce in metadata".into())
        })?;

        let nonce_vec = hex::decode(nonce_hex)
            .map_err(|e| SurrealFsError::Storage(format!("Invalid nonce hex: {}", e)))?;

        if nonce_vec.len() != 12 {
            return Err(SurrealFsError::Storage("Invalid nonce length".into()));
        }

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes.copy_from_slice(&nonce_vec);

        decrypt_bytes_with_aad(key, &raw_bytes, &nonce_bytes, norm_path.as_bytes())
            .or_else(|_| decrypt_bytes(key, &raw_bytes, &nonce_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [42u8; 32];
        let original = b"Highly confidential documents about Project Star!";

        let (ciphertext, nonce) = encrypt_bytes(&key, original).expect("encrypt");
        assert_ne!(ciphertext, original);

        let decrypted = decrypt_bytes(&key, &ciphertext, &nonce).expect("decrypt");
        assert_eq!(decrypted, original);

        // Tamper test
        let wrong_key = [99u8; 32];
        assert!(decrypt_bytes(&wrong_key, &ciphertext, &nonce).is_err());

        // AAD binding test: ciphertext moved to another path fails decryption
        let (ct_aad, nonce_aad) =
            encrypt_bytes_with_aad(&key, original, b"/vault/secret.txt").expect("encrypt_aad");
        assert!(
            decrypt_bytes_with_aad(&key, &ct_aad, &nonce_aad, b"/vault/other.txt").is_err(),
            "Mismatched AAD path must fail decryption"
        );
        let ok_aad =
            decrypt_bytes_with_aad(&key, &ct_aad, &nonce_aad, b"/vault/secret.txt").unwrap();
        assert_eq!(ok_aad, original);
    }
}
