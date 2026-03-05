use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use hmac::Hmac;
use sha2::Sha256;

/// Derive a 256-bit key from the machine's unique ID.
/// This ensures that the database is tied to the machine and cannot be easily
/// moved to another machine without the keys remaining accessible.
fn derive_key() -> Result<[u8; 32]> {
    let uid = machine_uid::get()
        .unwrap_or_else(|_| "cade-fallback-uid-if-machine-id-fails".to_string());
    
    let salt = b"cade-crypto-salt-v1";
    let mut key = [0u8; 32];
    
    pbkdf2::pbkdf2::<Hmac<Sha256>>(
        uid.as_bytes(),
        salt,
        100_000, // iterations
        &mut key
    ).map_err(|e| anyhow::anyhow!("PBKDF2 failed: {e}"))?;
    
    Ok(key)
}

/// Encrypt a string using AES-256-GCM.
/// Returns a base64-encoded string containing both the nonce and the ciphertext.
pub fn encrypt(plaintext: &str) -> Result<String> {
    let key_bytes = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {e}"))?;
    
    // Generate a unique 96-bit nonce for this encryption
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    
    let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {e}"))?;
    
    // Combine nonce + ciphertext and base64 encode
    let mut combined = nonce_bytes.to_vec();
    combined.extend(ciphertext);
    
    Ok(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, combined))
}

/// Decrypt a base64-encoded string using AES-256-GCM.
pub fn decrypt(encoded: &str) -> Result<String> {
    let key_bytes = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Cipher init failed: {e}"))?;
    
    let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .context("Base64 decode failed")?;
    
    if data.len() < 12 {
        anyhow::bail!("Invalid encrypted data: too short");
    }
    
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    
    let plaintext_bytes = cipher.decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption failed: {e}"))?;
    
    String::from_utf8(plaintext_bytes).context("UTF-8 decode failed")
}
