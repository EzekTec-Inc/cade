use crate::error::{Error, Result};
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use hmac::Hmac;
use sha2::Sha256;
use std::path::PathBuf;

// -- Key derivation

/// Resolve the on-disk DB-key path.  **Pure policy function** (no env, no
/// cwd, no I/O) so tests can exercise it without racing on process state.
///
/// Returns `Some(home.join(".cade").join("db.key"))` when `home` is
/// provided, `None` otherwise.  The caller decides how to obtain `home`
/// (typically `dirs::home_dir()`).
///
/// # P2-1
/// The previous implementation read `.cade-db.key` from the *process's
/// current working directory*.  That meant `cd`-ing into a hostile repo
/// (supply-chain, shared devcontainer, malicious git checkout) handed
/// the attacker the DB encryption key for every subsequent write.  The
/// new anchor is always `$HOME/.cade/db.key`; cwd is never consulted.
pub fn resolve_db_key_path(home: Option<PathBuf>) -> Option<PathBuf> {
    home.map(|h| h.join(".cade").join("db.key"))
}

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
    if let Ok(k) = std::env::var("CADE_DB_KEY") {
        return Ok(k);
    }
    if let Ok(k) = std::env::var("CADE_MACHINE_SECRET") {
        return Ok(k);
    }

    // P2-1: anchor the key file at $HOME/.cade/db.key.  The cwd-based
    // ./.cade-db.key path is NO LONGER read — it was a classic "trust
    // the current working directory" vulnerability.
    let Some(path) = resolve_db_key_path(dirs::home_dir()) else {
        return Err(Error::custom(
            "cannot resolve $HOME for DB key; set CADE_DB_KEY explicitly".to_string(),
        ));
    };

    if path.exists() {
        return std::fs::read_to_string(&path)
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                Error::custom(format!("Failed to read {}: {e}", path.display()))
            });
    }

    // Backwards compatibility: if cade.db exists beside the process, fall
    // back to machine_uid so existing local databases remain decryptable.
    // New installs never reach this branch because no cade.db is present.
    if std::path::Path::new("cade.db").exists()
        && let Ok(uid) = machine_uid::get()
    {
        tracing::warn!(
            "Using legacy machine_uid for database encryption. Consider migrating to CADE_DB_KEY."
        );
        return Ok(uid);
    }

    // Fresh install: generate a random key and persist it at the
    // canonical path with 0o600 perms on Unix.
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| Error::custom(format!("getrandom failed: {e}")))?;
    let secret = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key);

    // Ensure parent dir exists with tight perms (0o700 on Unix).
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(parent) {
                let mut perms = meta.permissions();
                perms.set_mode(0o700);
                let _ = std::fs::set_permissions(parent, perms);
            }
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
        {
            use std::io::Write;
            let _ = f.write_all(secret.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(&path, &secret);
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
    .map_err(|e| Error::custom(format!("PBKDF2 failed: {e}")))?;

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
        .map_err(|e| Error::custom(format!("getrandom (salt) failed: {e}")))?;

    let key_bytes = derive_key(&salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| Error::custom(format!("Cipher init failed: {e}")))?;

    // Generate a unique 96-bit nonce
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| Error::custom(format!("getrandom (nonce) failed: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| Error::custom(format!("Encryption failed: {e}")))?;

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

// endregion: --- Tests

pub fn decrypt(encoded: &str) -> Result<String> {
    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .map_err(|e| Error::custom(format!("Base64 decode failed: {e}")))?;

    // New format: salt(16) + nonce(12) + ciphertext = min 29 bytes
    // Legacy format: nonce(12) + ciphertext = min 13 bytes
    //
    // We distinguish by trying new format first (>= 29 bytes).
    if data.len() >= 29 {
        // New format — extract salt, derive key, decrypt
        let (salt, rest) = data.split_at(16);
        if rest.len() < 12 {
            return Err(Error::custom(
                "Invalid encrypted data: nonce too short".to_string(),
            ));
        }
        let (nonce_bytes, ciphertext) = rest.split_at(12);
        let key_bytes = derive_key(salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| Error::custom(format!("Cipher init failed: {e}")))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::custom(format!("Decryption failed: {e}")))?;
        return String::from_utf8(plaintext)
            .map_err(|e| Error::custom(format!("UTF-8 decode failed: {e}")));
    }

    // Legacy format — use the old static salt for backwards compatibility
    if data.len() >= 12 {
        let legacy_salt = b"cade-crypto-salt-v1";
        let key_bytes = derive_key(legacy_salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| Error::custom(format!("Cipher init (legacy) failed: {e}")))?;
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| Error::custom(format!("Legacy decryption failed: {e}")))?;
        tracing::warn!(
            "Decrypted a legacy (static-salt) API key — \
             re-save the provider to upgrade to the new format."
        );
        return String::from_utf8(plaintext)
            .map_err(|e| Error::custom(format!("UTF-8 decode failed: {e}")));
    }

    Err(Error::custom(format!(
        "Invalid encrypted data: too short ({} bytes)",
        data.len()
    )))
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

    /// Ensure all crypto tests use a stable key.  Setting `CADE_DB_KEY`
    /// is race-free across parallel tests because every test uses the
    /// same value and env mutation is idempotent.  This is also the
    /// P2-1-safe way to stub the key (no cwd file, no filesystem).
    static INIT: Once = Once::new();
    fn setup_test_key() {
        INIT.call_once(|| {
            // SAFETY: `std::env::set_var` is unsafe on edition 2024 because
            // it's not thread-safe on some platforms; `Once` guarantees we
            // set it exactly once before any test thread reads it.
            unsafe {
                std::env::set_var("CADE_DB_KEY", "test-crypto-secret-for-unit-tests");
            }
        });
    }

    // -- P2-1: resolve_db_key_path (pure policy)

    #[test]
    fn p2_1_resolves_to_dotcade_subdir() {
        let home = PathBuf::from("/home/alice");
        let got = resolve_db_key_path(Some(home));
        assert_eq!(got, Some(PathBuf::from("/home/alice/.cade/db.key")));
    }

    #[test]
    fn p2_1_none_when_home_unresolved() {
        let got = resolve_db_key_path(None);
        assert_eq!(got, None);
    }

    #[test]
    fn p2_1_windows_style_home() {
        let home = PathBuf::from(r"C:\Users\alice");
        let got = resolve_db_key_path(Some(home));
        // On Unix the separator is '/'; on Windows it's '\'.  Either way
        // the final component must be `db.key` and the one before must
        // be `.cade`.
        let p = got.unwrap();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("db.key"));
        assert_eq!(
            p.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str()),
            Some(".cade")
        );
    }

    // -- crypto round-trip (existing tests, unchanged behavior under P2-1)

    #[test]
    fn encrypt_decrypt_roundtrip() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let plaintext = "sk-ant-api03-very-secret-key-12345";

        // -- Exec
        let encrypted = encrypt(plaintext)?;

        // -- Check
        assert_ne!(encrypted, plaintext);
        assert!(encrypted.len() > plaintext.len());
        let decrypted = decrypt(&encrypted)?;
        assert_eq!(decrypted, plaintext);

        Ok(())
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let plaintext = "same-key-every-time";

        // -- Exec
        let enc1 = encrypt(plaintext)?;
        let enc2 = encrypt(plaintext)?;

        // -- Check
        assert_ne!(enc1, enc2);
        assert_eq!(decrypt(&enc1)?, plaintext);
        assert_eq!(decrypt(&enc2)?, plaintext);

        Ok(())
    }

    #[test]
    fn decrypt_invalid_base64_fails() {
        setup_test_key();
        // -- Exec & Check
        let result = decrypt("not-valid-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn decrypt_too_short_fails() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let short = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8; 5]);

        // -- Exec
        let result = decrypt(&short);

        // -- Check
        let msg = result.err().ok_or("Should be an error")?.to_string();
        assert!(msg.contains("too short"), "got: {msg}");

        Ok(())
    }

    #[test]
    fn decrypt_corrupted_data_fails() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let plaintext = "original-key";
        let encrypted = encrypt(plaintext)?;

        let mut chars: Vec<char> = encrypted.chars().collect();
        if chars.len() > 20 {
            chars[20] = if chars[20] == 'A' { 'B' } else { 'A' };
        }
        let corrupted: String = chars.into_iter().collect();

        // -- Exec & Check
        let result = decrypt(&corrupted);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn encrypt_empty_string() -> Result<()> {
        setup_test_key();
        // -- Exec
        let encrypted = encrypt("")?;
        let decrypted = decrypt(&encrypted)?;

        // -- Check
        assert_eq!(decrypted, "");

        Ok(())
    }

    #[test]
    fn encrypt_unicode_content() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let plaintext = "日本語のAPIキー🔑";

        // -- Exec
        let encrypted = encrypt(plaintext)?;
        let decrypted = decrypt(&encrypted)?;

        // -- Check
        assert_eq!(decrypted, plaintext);

        Ok(())
    }

    #[test]
    fn encrypt_long_key() -> Result<()> {
        setup_test_key();
        // -- Setup & Fixtures
        let plaintext = "a".repeat(10_000);

        // -- Exec
        let encrypted = encrypt(&plaintext)?;
        let decrypted = decrypt(&encrypted)?;

        // -- Check
        assert_eq!(decrypted, plaintext);

        Ok(())
    }
}
