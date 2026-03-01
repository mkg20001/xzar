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

/// Parse a hash string that can be in either:
/// - SRI format: sha256-BASE64...
/// - Nix format: sha256:NIXBASE32...
///
/// Returns (algorithm, nix_base32_hash)
pub fn parse_hash(hash_str: &str) -> Result<(String, String)> {
    // Try SRI format first (sha256-base64)
    if hash_str.contains('-') {
        return sri_to_nix_hash(hash_str);
    }

    // Try Nix format (sha256:nixbase32)
    if hash_str.contains(':') {
        return nix_hash_to_parts(hash_str);
    }

    Err(AppError::Crypto(format!(
        "Invalid hash format: {}. Expected SRI (sha256-base64) or Nix (sha256:base32)",
        hash_str
    )))
}

/// Parse Nix format hash (sha256:nixbase32) into parts
pub fn nix_hash_to_parts(nix_hash: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = nix_hash.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(AppError::Crypto(format!("Invalid Nix hash: {}", nix_hash)));
    }

    let algo = parts[0];
    let hash = parts[1];

    if algo != "sha256" && algo != "sha512" && algo != "sha1" {
        return Err(AppError::Crypto(format!("Unsupported hash algorithm: {}", algo)));
    }

    // Validate it looks like nix base32
    let valid_chars = "0123456789abcdfghijklmnpqrsvwxyz";
    if !hash.chars().all(|c| valid_chars.contains(c)) {
        return Err(AppError::Crypto(format!("Invalid Nix base32 hash: {}", hash)));
    }

    Ok((algo.to_string(), hash.to_string()))
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
    use ed25519_dalek::{SigningKey, Verifier};

    #[test]
    fn test_nix_base32_zeros() {
        // All zeros should produce all '0's in nix base32
        let bytes = [0u8; 32];
        let result = bytes_to_nix_base32(&bytes);
        assert_eq!(result.len(), 52); // SHA256 produces 52 char base32
        assert!(result.chars().all(|c| c == '0'));
    }

    #[test]
    fn test_nix_base32_ones() {
        // All 0xFF should produce all 'z's in nix base32
        let bytes = [0xFFu8; 32];
        let result = bytes_to_nix_base32(&bytes);
        assert_eq!(result.len(), 52);
        // Last char depends on padding, but most should be 'z'
        assert!(result.chars().filter(|&c| c == 'z').count() > 45);
    }

    #[test]
    fn test_nix_base32_length() {
        // Test various hash sizes
        assert_eq!(nix_base32_len(20), 32);  // SHA1
        assert_eq!(nix_base32_len(32), 52);  // SHA256
        assert_eq!(nix_base32_len(64), 103); // SHA512
    }

    #[test]
    fn test_nix_base32_alphabet() {
        // Verify output only uses valid nix base32 chars
        let bytes: Vec<u8> = (0..32).collect();
        let result = bytes_to_nix_base32(&bytes);
        let valid_chars = "0123456789abcdfghijklmnpqrsvwxyz";
        assert!(result.chars().all(|c| valid_chars.contains(c)));
    }

    #[test]
    fn test_sri_to_nix_hash_sha256() {
        let (algo, hash) = sri_to_nix_hash("sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
            .unwrap();
        assert_eq!(algo, "sha256");
        assert_eq!(hash.len(), 52);
    }

    #[test]
    fn test_sri_to_nix_hash_sha512() {
        // SHA512 all zeros
        let b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
        let (algo, hash) = sri_to_nix_hash(&format!("sha512-{}", b64)).unwrap();
        assert_eq!(algo, "sha512");
        assert_eq!(hash.len(), 103);
    }

    #[test]
    fn test_sri_to_nix_hash_invalid() {
        assert!(sri_to_nix_hash("invalid").is_err());
        assert!(sri_to_nix_hash("md5-AAAA").is_err());
        assert!(sri_to_nix_hash("sha256-!!!invalid!!!").is_err());
    }

    #[test]
    fn test_base64_to_nix_base32() {
        let result = base64_to_nix_base32("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        assert_eq!(result.len(), 52);
    }

    #[test]
    fn test_fingerprint_no_refs() {
        let fp = fingerprint(
            "abc123-test-1.0",
            "sha256:0000000000000000000000000000000000000000000000000000",
            12345,
            &[],
        );
        assert_eq!(
            fp,
            "1;/nix/store/abc123-test-1.0;sha256:0000000000000000000000000000000000000000000000000000;12345;"
        );
    }

    #[test]
    fn test_fingerprint_with_refs() {
        let fp = fingerprint(
            "abc123-test-1.0",
            "sha256:0000000000000000000000000000000000000000000000000000",
            12345,
            &["ref1-lib-1.0".to_string(), "ref2-lib-2.0".to_string()],
        );
        assert!(fp.contains("/nix/store/ref1-lib-1.0"));
        assert!(fp.contains("/nix/store/ref2-lib-2.0"));
        assert!(fp.contains(","));
    }

    #[test]
    fn test_signing_key_from_config() {
        // Generate a test key
        let secret = [0x42u8; 32];
        let _signing_key = SigningKey::from_bytes(&secret);
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("test-key-1:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();
        assert_eq!(nix_key.name, "test-key-1");
    }

    #[test]
    fn test_signing_key_invalid_format() {
        assert!(NixSigningKey::from_config("no-colon").is_err());
        assert!(NixSigningKey::from_config("key:!!!invalid-base64!!!").is_err());
        assert!(NixSigningKey::from_config("key:AAAA").is_err()); // Too short
    }

    #[test]
    fn test_sign_message() {
        let secret = [0x42u8; 32];
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("test-key:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();
        let signature = nix_key.sign("hello world");

        // Verify format: "keyname:base64signature"
        assert!(signature.starts_with("test-key:"));
        let sig_b64 = signature.strip_prefix("test-key:").unwrap();
        let sig_bytes = BASE64_STANDARD.decode(sig_b64).unwrap();
        assert_eq!(sig_bytes.len(), 64); // Ed25519 signature is 64 bytes
    }

    #[test]
    fn test_sign_and_verify() {
        use ed25519_dalek::Signature;

        // Generate a test key
        let secret = [0x42u8; 32];
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("test-key:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();

        // Sign a message
        let message = "test message to sign";
        let signature_str = nix_key.sign(message);

        // Extract and verify signature
        let sig_b64 = signature_str.strip_prefix("test-key:").unwrap();
        let sig_bytes = BASE64_STANDARD.decode(sig_b64).unwrap();
        let signature = Signature::from_slice(&sig_bytes).unwrap();

        // Verify should succeed
        assert!(verifying_key.verify(message.as_bytes(), &signature).is_ok());

        // Verify with wrong message should fail
        assert!(verifying_key.verify(b"wrong message", &signature).is_err());
    }

    #[test]
    fn test_sign_drv() {
        let secret = [0x42u8; 32];
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("cache.example.com-1:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();

        let signature = nix_key.sign_drv(
            "abc123-package-1.0",
            "sha256:0000000000000000000000000000000000000000000000000000",
            1024,
            &["dep1-lib".to_string()],
        );

        assert!(signature.starts_with("cache.example.com-1:"));
        let sig_b64 = signature.strip_prefix("cache.example.com-1:").unwrap();
        assert!(BASE64_STANDARD.decode(sig_b64).is_ok());
    }

    #[test]
    fn test_signature_deterministic() {
        let secret = [0x42u8; 32];
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("test:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();

        let sig1 = nix_key.sign("same message");
        let sig2 = nix_key.sign("same message");

        // Ed25519 signatures are deterministic
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_different_messages_different_signatures() {
        let secret = [0x42u8; 32];
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let config_str = format!("test:{}", key_b64);

        let nix_key = NixSigningKey::from_config(&config_str).unwrap();

        let sig1 = nix_key.sign("message 1");
        let sig2 = nix_key.sign("message 2");

        assert_ne!(sig1, sig2);
    }
}
