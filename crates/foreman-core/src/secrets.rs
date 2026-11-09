//! At-rest encryption for secrets stored in the Foreman DB (SSH private key, SSH passwords).
//!
//! A random 256-bit master key lives in `<data_dir>/secret.key` (created once, mode 0600 on
//! Unix), outside the SQLite file, so a copied DB alone does not reveal secrets.
//! Values are sealed with AES-256-GCM and stored as `enc:v1:<base64(nonce || ciphertext)>`.

use std::path::Path;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

use crate::error::AppError;

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

/// Holds the master key and seals/opens secret strings.
#[derive(Clone)]
pub struct SecretBox {
    key: [u8; 32],
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(..)")
    }
}

impl SecretBox {
    /// Build from raw key bytes (tests).
    pub fn from_key(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// Load the master key from `path`, creating it with fresh random bytes if missing.
    pub fn load_or_create(path: &Path) -> Result<Self, AppError> {
        if path.exists() {
            let hex = std::fs::read_to_string(path)
                .map_err(|e| AppError::Internal(format!("read {}: {e}", path.display())))?;
            let bytes = hex::decode(hex.trim())
                .map_err(|_| AppError::Internal(format!("{} is corrupt", path.display())))?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| AppError::Internal(format!("{} has wrong length", path.display())))?;
            return Ok(Self { key });
        }

        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| AppError::Internal(format!("getrandom: {e}")))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("create {}: {e}", parent.display())))?;
        }
        std::fs::write(path, hex::encode(key))
            .map_err(|e| AppError::Internal(format!("write {}: {e}", path.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| AppError::Internal(format!("chmod {}: {e}", path.display())))?;
        }
        Ok(Self { key })
    }

    /// Encrypt a UTF-8 secret.
    pub fn seal(&self, plaintext: &str) -> Result<String, AppError> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::Internal(format!("cipher init: {e}")))?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|e| AppError::Internal(format!("getrandom: {e}")))?;
        let nonce = Nonce::from(nonce_bytes);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| AppError::Internal("encrypt failed".into()))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ct);
        Ok(format!("{PREFIX}{}", B64.encode(out)))
    }

    /// Decrypt a value produced by [`seal`](Self::seal).
    pub fn open(&self, sealed: &str) -> Result<String, AppError> {
        let b64 = sealed
            .strip_prefix(PREFIX)
            .ok_or_else(|| AppError::Internal("secret is not encrypted".into()))?;
        let raw = B64
            .decode(b64)
            .map_err(|e| AppError::Internal(format!("secret base64: {e}")))?;
        if raw.len() < NONCE_LEN {
            return Err(AppError::Internal("secret too short".into()));
        }
        let (nonce_bytes, ct) = raw.split_at(NONCE_LEN);
        let nonce_arr: [u8; NONCE_LEN] = nonce_bytes
            .try_into()
            .map_err(|_| AppError::Internal("bad nonce".into()))?;
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| AppError::Internal(format!("cipher init: {e}")))?;
        let pt = cipher
            .decrypt(&Nonce::from(nonce_arr), ct)
            .map_err(|_| AppError::Internal("decrypt failed (wrong secret.key?)".into()))?;
        String::from_utf8(pt).map_err(|e| AppError::Internal(format!("secret utf-8: {e}")))
    }

    /// True if the value carries the encrypted prefix.
    pub fn is_sealed(value: &str) -> bool {
        value.starts_with(PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let sb = SecretBox::from_key([7u8; 32]);
        let sealed = sb.seal("hunter2").expect("seal");
        assert!(SecretBox::is_sealed(&sealed));
        assert!(!sealed.contains("hunter2"));
        assert_eq!(sb.open(&sealed).expect("open"), "hunter2");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = SecretBox::from_key([1u8; 32]).seal("x").expect("seal");
        assert!(SecretBox::from_key([2u8; 32]).open(&sealed).is_err());
    }

    #[test]
    fn key_file_persists() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("secret.key");
        let a = SecretBox::load_or_create(&path).expect("create");
        let sealed = a.seal("pw").expect("seal");
        let b = SecretBox::load_or_create(&path).expect("load");
        assert_eq!(b.open(&sealed).expect("open"), "pw");
    }
}
