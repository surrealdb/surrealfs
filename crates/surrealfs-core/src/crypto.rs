//! End-to-end Client-Side Encryption Envelope (§24.3).

use crate::errors::{Result, SurrealFsError};
use crate::fs::SurrealFs;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde_json::json;

/// Encrypt raw data using AES-256-GCM.
/// Returns `(ciphertext, nonce_bytes)`.
pub fn encrypt_bytes(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12])> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| SurrealFsError::Other(format!("RNG error: {}", e)))?;
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| SurrealFsError::Storage(format!("Encryption failed: {}", e)))?;

    Ok((ciphertext, nonce_bytes))
}

/// Decrypt AES-256-GCM ciphertext using the given key and nonce.
pub fn decrypt_bytes(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8; 12]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from(*nonce);

    let plaintext = cipher.decrypt(&nonce, ciphertext).map_err(|e| {
        SurrealFsError::PermissionDenied(format!("Decryption failed or invalid key: {}", e))
    })?;

    Ok(plaintext)
}

/// Extension methods on SurrealFs for encrypted file operations.
impl SurrealFs {
    /// Write an encrypted file into SurrealFS with metadata `encrypted = true`.
    pub async fn write_encrypted(
        &self,
        path: &str,
        data: &[u8],
        key: &[u8; 32],
        key_id: Option<&str>,
    ) -> Result<()> {
        let (ciphertext, nonce) = encrypt_bytes(key, data)?;
        let nonce_hex = hex::encode(nonce);

        // Write ciphertext
        self.write_bytes(path, &ciphertext, None).await?;

        // Update metadata to record encryption parameters
        let sql = "UPDATE fn::sfs_resolve($path) SET meta.encrypted = true, meta.encryption_algo = 'aes-256-gcm', meta.nonce = $nonce, meta.key_id = $key_id;";
        self.db
            .query(sql)
            .bind(("path", path.to_string()))
            .bind(("nonce", nonce_hex))
            .bind(("key_id", key_id.map(|s| s.to_string())))
            .await?;

        Ok(())
    }

    /// Read and decrypt an encrypted file from SurrealFS.
    pub async fn read_encrypted(&self, path: &str, key: &[u8; 32]) -> Result<Vec<u8>> {
        let entry = self
            .stat(path)
            .await?
            .ok_or_else(|| SurrealFsError::NotFound(format!("File {} not found", path)))?;
        let meta = entry.meta.unwrap_or(json!({}));

        let is_encrypted = meta
            .get("encrypted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !is_encrypted {
            return self.read_bytes(path).await;
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

        let ciphertext = self.read_bytes(path).await?;
        decrypt_bytes(key, &ciphertext, &nonce_bytes)
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
    }
}
