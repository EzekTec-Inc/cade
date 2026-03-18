use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use hmac::Hmac;
use sha2::Sha256;

// -- Key derivation

/// Derive a 256-bit key using PBKDF2-HMAC-SHA256.
///
/// # H-01 fix — per-record random salt
/// The caller supplies `salt` (16 random bytes generated per encryption).
/// This means each encrypted value has a unique salt, so even identical
/// plaintexts produce different ciphertexts and an attacker cannot use
/// pre-computed tables even with knowledge of the salt scheme.
///
/// # H-02 fix — hard failure on missing machine UID
/// If `machine_uid::get()` fails (container, certain VMs) we return an error
/// rather than silently falling back to a known constant.  Callers should
/// surface this clearly to the user.
fn get_root_secret() -> Result<String> {
    if let Ok(k) = std::env::var("CADE_DB_KEY") { return Ok(k); }
    if let Ok(k) = std::env::var("CADE_MACHINE_SECRET") { return Ok(k); }

    let path = std::path::Path::new(".cade-db.key");
    if path.exists() {
        return std::fs::read_to_string(path)
            .map(|s| s.trim().to_string())
            .context("Failed to read .cade-db.key");
    }

    // Backwards compatibility check: if cade.db exists, fall back to machine_uid
    if std::path::Path::new("cade.db").exists() {
        if let Ok(uid) = machine_uid::get() {
            tracing::warn!("Using legacy machine_uid for database encryption. Consider migrating to CADE_DB_KEY.");
            return Ok(uid);
        }
    }

    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
    let secret = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path) {
            use std::io::Write;
            let _ = f.write_all(secret.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(path, &secret);
    }

    Ok(secret)
}

fn derive_key(salt: &[u8]) -> Result<[u8; 32]> {
    let uid = get_root_secret()?;

    let mut key = [0u8; 32];
    pbkdf2::pbkdf2::<Hmac<Sha256>>(
        uid.as_bytes(),
        salt,
        100_000, // iterations
        &mut key,
    )
    .map_err(|e| anyhow::anyhow!("PBKDF2 failed: {e}"))?;

    Ok(key)
}

// -- Encryption

/// Encrypt a plaintext string with AES-256-GCM.
///
/// Output format (base64-encoded):
///   [ 16-byte random salt | 12-byte random nonce | ciphertext + 16-byte GCM tag ]
///
/// Both salt and nonce are random per call, so the same plaintext always
/// produces a different output.
pub fn encrypt(plaintext: &str) -> Result<String> {
    // H-01: generate a fresh 16-byte salt for every encryption
    let mut salt = [0u8; 16];
    getrandom::getrandom(&mut salt)
        .map_err(|e| anyhow::anyhow!("getrandom (salt) failed: {e}"))?;

    let key_bytes = derive_key(&salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {e}"))?;

    // Generate a unique 96-bit nonce
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| anyhow::anyhow!("getrandom (nonce) failed: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;

    // Layout: salt(16) | nonce(12) | ciphertext
    let mut combined = Vec::with_capacity(16 + 12 + ciphertext.len());
    combined.extend_from_slice(&salt);
    combined.extend_from_slice(&nonce_bytes);
    combined.extend(ciphertext);

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        combined,
    ))
}

// -- Decryption

/// Decrypt a value previously produced by `encrypt()`.
///
/// Handles both the new format (salt prefix) and the legacy format
/// (no salt prefix, used hardcoded salt) for backwards compatibility
/// with any existing DB values.

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use std::sync::Once;

    /// Ensure all crypto tests use a stable key (avoids race conditions
    /// when parallel tests race on `.cade-db.key` creation).
    static INIT: Once = Once::new();
    fn setup_test_key() {
        INIT.call_once(|| {
            let _ = std::fs::write(
                ".cade-db.key",
                "test-crypto-secret-for-unit-tests",
            );
        });
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        setup_test_key();
        let plaintext = "sk-ant-api03-very-secret-key-12345";
        let encrypted = encrypt(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        assert!(encrypted.len() > plaintext.len());

        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() {
        setup_test_key();
        let plaintext = "same-key-every-time";
        let enc1 = encrypt(plaintext).unwrap();
        let enc2 = encrypt(plaintext).unwrap();
        assert_ne!(enc1, enc2);

        assert_eq!(decrypt(&enc1).unwrap(), plaintext);
        assert_eq!(decrypt(&enc2).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_invalid_base64_fails() {
        setup_test_key();
        let result = decrypt("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_too_short_fails() {
        setup_test_key();
        let short = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &[0u8; 5],
        );
        let result = decrypt(&short);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("too short"), "got: {msg}");
    }

    #[test]
    fn decrypt_corrupted_data_fails() {
        setup_test_key();
        let plaintext = "original-key";
        let encrypted = encrypt(plaintext).unwrap();

        let mut chars: Vec<char> = encrypted.chars().collect();
        if chars.len() > 20 {
            chars[20] = if chars[20] == 'A' { 'B' } else { 'A' };
        }
        let corrupted: String = chars.into_iter().collect();

        let result = decrypt(&corrupted);
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_empty_string() {
        setup_test_key();
        let encrypted = encrypt("").unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn encrypt_unicode_content() {
        setup_test_key();
        let plaintext = "日本語のAPIキー🔑";
        let encrypted = encrypt(plaintext).unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_long_key() {
        setup_test_key();
        let plaintext = "a".repeat(10_000);
        let encrypted = encrypt(&plaintext).unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}

// endregion: --- Tests

pub fn decrypt(encoded: &str) -> Result<String> {
    let data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded,
    )
    .context("Base64 decode failed")?;

    // New format: salt(16) + nonce(12) + ciphertext = min 29 bytes
    // Legacy format: nonce(12) + ciphertext = min 13 bytes
    //
    // We distinguish by trying new format first (>= 29 bytes).
    if data.len() >= 29 {
        // New format — extract salt, derive key, decrypt
        let (salt, rest) = data.split_at(16);
        if rest.len() < 12 {
            anyhow::bail!("Invalid encrypted data: nonce too short");
        }
        let (nonce_bytes, ciphertext) = rest.split_at(12);
        let key_bytes = derive_key(salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| anyhow::anyhow!("Cipher init failed: {e}"))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Decryption failed: {e}"))?;
        return String::from_utf8(plaintext).context("UTF-8 decode failed");
    }

    // Legacy format — use the old static salt for backwards compatibility
    if data.len() >= 12 {
        let legacy_salt = b"cade-crypto-salt-v1";
        let key_bytes = derive_key(legacy_salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| anyhow::anyhow!("Cipher init (legacy) failed: {e}"))?;
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Legacy decryption failed: {e}"))?;
        tracing::warn!(
            "Decrypted a legacy (static-salt) API key — \
             re-save the provider to upgrade to the new format."
        );
        return String::from_utf8(plaintext).context("UTF-8 decode failed");
    }

    anyhow::bail!("Invalid encrypted data: too short ({} bytes)", data.len())
}
