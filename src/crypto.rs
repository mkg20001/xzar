use base64::prelude::*;
use ed25519_dalek::{SecretKey, Signer, SigningKey};

use crate::error::{AppError, Result};

// Nix uses a custom base32 alphabet (no e, o, t, u)
const NIX_BASE32: &[u8; 32] = b"0123456789abcdfghijklmnpqrsvwxyz";

/// Convert a byte array to Nix's custom base32 encoding
pub fn bytes_to_nix_base32(bytes: &[u8]) -> String {
    let hash_size = bytes.len();
    let len = nix_base32_len(hash_size);
    let mut result = String::with_capacity(len);

    for n in (0..len).rev() {
        let b = n * 5;
        let i = b / 8;
        let j = b % 8;

        let c = if i < hash_size {
            let mut val = (bytes[i] >> j) as usize;
            if i + 1 < hash_size {
                val |= ((bytes[i + 1] as usize) << (8 - j)) & 0xFF;
            }
            val & 0x1F
        } else {
            0
        };

        result.push(NIX_BASE32[c] as char);
    }

    result
}

fn nix_base32_len(hash_size: usize) -> usize {
    (hash_size * 8 + 4) / 5
}

/// Convert SRI hash (sha256-base64) to Nix format (sha256:base32)
pub fn sri_to_nix_hash(integrity: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = integrity.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(AppError::Crypto(format!("Invalid SRI hash: {}", integrity)));
    }

    let algo = parts[0];
    let b64_hash = parts[1];

    if algo != "sha256" && algo != "sha512" && algo != "sha1" {
        return Err(AppError::Crypto(format!("Unsupported hash algorithm: {}", algo)));
    }

    let hash_bytes = BASE64_STANDARD
        .decode(b64_hash)
        .map_err(|e| AppError::Crypto(format!("Invalid base64: {}", e)))?;

    let nix_hash = bytes_to_nix_base32(&hash_bytes);

    Ok((algo.to_string(), nix_hash))
}

/// Convert base64 to Nix base32
pub fn base64_to_nix_base32(b64: &str) -> Result<String> {
    let bytes = BASE64_STANDARD
        .decode(b64)
        .map_err(|e| AppError::Crypto(format!("Invalid base64: {}", e)))?;

    Ok(bytes_to_nix_base32(&bytes))
}

/// Format a Nix store path
pub fn print_store_path(drv_full: &str) -> String {
    format!("/nix/store/{}", drv_full)
}

/// Create a fingerprint string for signing
/// Format: "1;/nix/store/{drvFull};{hash};{size};{comma-separated references}"
pub fn fingerprint(drv_full: &str, nar_hash: &str, nar_size: i64, references: &[String]) -> String {
    let refs_str: String = references
        .iter()
        .map(|r| print_store_path(r))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "1;{};{};{};{}",
        print_store_path(drv_full),
        nar_hash,
        nar_size,
        refs_str
    )
}

/// Signing key parsed from config format "keyname:base64privatekey"
pub struct NixSigningKey {
    pub name: String,
    signing_key: SigningKey,
}

impl NixSigningKey {
    pub fn from_config(key_string: &str) -> Result<Self> {
        let parts: Vec<&str> = key_string.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(AppError::Crypto("Invalid signing key format".to_string()));
        }

        let name = parts[0].to_string();
        let key_bytes = BASE64_STANDARD
            .decode(parts[1])
            .map_err(|e| AppError::Crypto(format!("Invalid key base64: {}", e)))?;

        // Ed25519 secret key is 64 bytes (includes public key)
        // We need the first 32 bytes as the secret key
        if key_bytes.len() < 32 {
            return Err(AppError::Crypto("Signing key too short".to_string()));
        }

        let secret_key: SecretKey = key_bytes[..32]
            .try_into()
            .map_err(|_| AppError::Crypto("Invalid secret key length".to_string()))?;

        let signing_key = SigningKey::from_bytes(&secret_key);

        Ok(Self { name, signing_key })
    }

    /// Sign a message and return "{keyname}:{base64signature}"
    pub fn sign(&self, message: &str) -> String {
        let signature = self.signing_key.sign(message.as_bytes());
        let sig_base64 = BASE64_STANDARD.encode(signature.to_bytes());
        format!("{}:{}", self.name, sig_base64)
    }

    /// Sign a derivation and return the full signature
    pub fn sign_drv(
        &self,
        drv_full: &str,
        nar_hash: &str,
        nar_size: i64,
        references: &[String],
    ) -> String {
        let fp = fingerprint(drv_full, nar_hash, nar_size, references);
        self.sign(&fp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nix_base32() {
        // Test vector: sha256 hash converted to nix base32
        let bytes = [0u8; 32]; // All zeros
        let result = bytes_to_nix_base32(&bytes);
        assert_eq!(result.len(), 52); // SHA256 produces 52 char base32
    }

    #[test]
    fn test_sri_to_nix_hash() {
        // sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= (all zeros)
        let (algo, hash) = sri_to_nix_hash("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
            .unwrap();
        assert_eq!(algo, "sha256");
        assert_eq!(hash.len(), 52);
    }
}
