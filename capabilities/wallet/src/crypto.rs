use anyhow::{anyhow, Result};
use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

/// Derive a 32-byte encryption key from a passphrase + salt using Argon2id.
fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<[u8; 32]> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|e| anyhow!("argon2 KDF failed: {}", e))?;
    Ok(key)
}

/// Encrypt a secret with a passphrase.
///
/// Output format: `[salt:16][nonce:12][ciphertext+tag]`
pub fn encrypt_secret(plaintext: &[u8], passphrase: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| anyhow!("cipher init: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("encryption failed: {}", e))?;

    let mut output = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

/// Decrypt a secret with a passphrase.
///
/// Expects input format: `[salt:16][nonce:12][ciphertext+tag]`
pub fn decrypt_secret(encrypted: &[u8], passphrase: &[u8]) -> Result<Vec<u8>> {
    if encrypted.len() < SALT_LEN + NONCE_LEN + 1 {
        return Err(anyhow!("encrypted data too short"));
    }

    let salt = &encrypted[..SALT_LEN];
    let nonce_bytes = &encrypted[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &encrypted[SALT_LEN + NONCE_LEN..];

    let key = derive_key(passphrase, salt)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)
        .map_err(|e| anyhow!("cipher init: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("decryption failed: wrong passphrase or corrupted data"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let plaintext = b"my-secret-private-key-bytes-here";
        let passphrase = b"hunter2";

        let encrypted = encrypt_secret(plaintext, passphrase).unwrap();
        let decrypted = decrypt_secret(&encrypted, passphrase).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_passphrase() {
        let plaintext = b"secret";
        let encrypted = encrypt_secret(plaintext, b"correct").unwrap();
        let result = decrypt_secret(&encrypted, b"wrong");
        assert!(result.is_err());
    }

    #[test]
    fn corrupted_data() {
        let plaintext = b"secret";
        let mut encrypted = encrypt_secret(plaintext, b"pass").unwrap();
        // Flip a byte in the ciphertext
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xff;
        let result = decrypt_secret(&encrypted, b"pass");
        assert!(result.is_err());
    }

    #[test]
    fn too_short() {
        let result = decrypt_secret(&[0u8; 10], b"pass");
        assert!(result.is_err());
    }

    #[test]
    fn different_encryptions_differ() {
        let plaintext = b"same-data";
        let pass = b"same-pass";
        let enc1 = encrypt_secret(plaintext, pass).unwrap();
        let enc2 = encrypt_secret(plaintext, pass).unwrap();
        // Random salt+nonce means different ciphertexts
        assert_ne!(enc1, enc2);
        // But both decrypt to the same plaintext
        assert_eq!(decrypt_secret(&enc1, pass).unwrap(), plaintext);
        assert_eq!(decrypt_secret(&enc2, pass).unwrap(), plaintext);
    }
}
