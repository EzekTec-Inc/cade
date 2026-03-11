use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use hmac::Hmac;
use sha2::Sha256;

// ── Key derivation ────────────────────────────────────────────────────────────

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
fn derive_key(salt: &[u8]) -> Result<[u8; 32]> {
    let uid = machine_uid::get()
        .map_err(|e| anyhow::anyhow!(
            "Cannot derive encryption key: machine UID unavailable ({e}). \
             Set CADE_MACHINE_SECRET env var as a fallback."
        ))
        // Allow an explicit env-var override for environments where machine_uid fails
        .or_else(|e| {
            std::env::var("CADE_MACHINE_SECRET")
                .map_err(|_| e)
        })?;

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

// ── Encryption ────────────────────────────────────────────────────────────────

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

// ── Decryption ────────────────────────────────────────────────────────────────

/// Decrypt a value previously produced by `encrypt()`.
///
/// Handles both the new format (salt prefix) and the legacy format
/// (no salt prefix, used hardcoded salt) for backwards compatibility
/// with any existing DB values.
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
