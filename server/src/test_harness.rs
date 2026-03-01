//! Test harness for xzar-server integration tests
//!
//! Provides utilities for bootstrapping test servers with proper credentials,
//! database setup, and cleanup.

use std::sync::atomic::{AtomicU16, Ordering};

use crate::auth::TokenStore;
use crate::config::{Config, DbConfig, RocketConfig, TokenConfig};
use crate::crypto::NixSigningKey;
use crate::db::Database;
use crate::storage::Storage;

use base64::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use ed25519_dalek::SigningKey;
use rand::RngCore;
use rocket::local::asynchronous::Client;
use sha2::{Digest, Sha512};
use tempfile::TempDir;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(18080);

/// Test credentials for authentication
#[derive(Clone, Debug)]
pub struct TestCredentials {
    /// The raw upload token (secret)
    pub upload_token: String,
    /// SHA-512 hash of the token (hex encoded, stored server-side)
    pub token_hash: String,
    /// Ed25519 signing key in "name:base64key" format
    pub signing_key: String,
    /// Just the key name
    pub key_name: String,
}

impl TestCredentials {
    /// Generate new random test credentials
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();

        // Generate random upload token
        let mut token_bytes = [0u8; 32];
        rng.fill_bytes(&mut token_bytes);
        let upload_token = BASE64_STANDARD.encode(token_bytes);

        // Hash the token (hex encoded like the server does)
        let mut hasher = Sha512::new();
        hasher.update(upload_token.as_bytes());
        let hash_bytes = hasher.finalize();
        let token_hash = hex_encode(&hash_bytes);

        // Generate random Ed25519 signing key
        let mut secret = [0u8; 32];
        rng.fill_bytes(&mut secret);
        let key_b64 = BASE64_STANDARD.encode(&secret);
        let key_name = "test-cache-1".to_string();
        let signing_key = format!("{}:{}", key_name, key_b64);

        Self {
            upload_token,
            token_hash,
            signing_key,
            key_name,
        }
    }

    /// Get the NixSigningKey from these credentials
    pub fn nix_signing_key(&self) -> NixSigningKey {
        NixSigningKey::from_config(&self.signing_key).unwrap()
    }

    /// Get the public key in Nix format (keyname:base64pubkey)
    /// Used for trusted-public-keys configuration
    pub fn public_key(&self) -> String {
        // Parse the signing key to get the secret bytes
        let parts: Vec<&str> = self.signing_key.splitn(2, ':').collect();
        let secret_bytes = BASE64_STANDARD.decode(parts[1]).unwrap();

        // Create signing key and get verifying (public) key
        let secret: [u8; 32] = secret_bytes[..32].try_into().unwrap();
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        // Format as "keyname:base64(pubkey)"
        let pub_b64 = BASE64_STANDARD.encode(verifying_key.as_bytes());
        format!("{}:{}", self.key_name, pub_b64)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX_CHARS[(byte >> 4) as usize] as char);
        hex.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    hex
}

/// Test server configuration and state
pub struct TestServer {
    /// Rocket client for making requests
    pub client: Client,
    /// Test credentials
    pub credentials: TestCredentials,
    /// Temp directory for storage (kept alive for duration)
    _storage_dir: TempDir,
    /// Server port
    pub port: u16,
}

/// Builder for creating test servers
pub struct TestServerBuilder {
    credentials: Option<TestCredentials>,
    database_url: Option<String>,
}

impl Default for TestServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestServerBuilder {
    pub fn new() -> Self {
        Self {
            credentials: None,
            database_url: None,
        }
    }

    /// Use specific credentials instead of generating new ones
    pub fn with_credentials(mut self, credentials: TestCredentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Use specific database URL
    pub fn with_database_url(mut self, url: String) -> Self {
        self.database_url = Some(url);
        self
    }

    /// Build and start the test server
    pub async fn build(self) -> TestServer {
        let credentials = self.credentials.unwrap_or_else(TestCredentials::generate);

        // Create temp storage directory
        let storage_dir = TempDir::new().expect("Failed to create temp storage dir");
        let storage_path = storage_dir.path().to_string_lossy().to_string();

        // Get database URL from env or builder
        let database_url = self.database_url.unwrap_or_else(|| {
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
                std::env::var("DATABASE_URL")
                    .expect("TEST_DATABASE_URL or DATABASE_URL must be set")
            })
        });

        // Get unique port
        let port = PORT_COUNTER.fetch_add(1, Ordering::SeqCst);

        // Create config
        let config = Config {
            rocket: RocketConfig {
                host: "127.0.0.1".to_string(),
                port,
            },
            storage: storage_path.clone(),
            tokens: vec![TokenConfig::Hashed {
                hashed: credentials.token_hash.clone(),
            }],
            signing_key: Some(credentials.signing_key.clone()),
            signing_pub_key: None,
            external_url: None,
            db: DbConfig {
                client: "pg".to_string(),
                connection: database_url.clone(),
            },
            sentry_dsn: None,
        };

        // Initialize database pool
        let manager = ConnectionManager::<PgConnection>::new(&database_url);
        let pool = Pool::builder()
            .max_size(5)
            .build(manager)
            .expect("Failed to create test database pool");

        // Run migrations
        {
            let mut conn = pool.get().expect("Failed to get connection for migrations");
            crate::db::run_migrations(&mut conn);
        }

        // Initialize storage
        let storage = Storage::new(&storage_path)
            .await
            .expect("Failed to initialize test storage");

        // Initialize token store
        let token_hashes = config.get_token_hashes();
        let token_store = TokenStore::new(token_hashes);

        // Build rocket instance without starting GC
        let rocket = rocket::build()
            .manage(Database(pool))
            .manage(storage)
            .manage(token_store)
            .manage(config)
            .mount(
                "/",
                rocket::routes![
                    crate::routes::nix_cache_info,
                    crate::routes::get_narinfo,
                    crate::routes::get_nar,
                    crate::routes::check_paths,
                    crate::routes::lock_request,
                    crate::routes::lock_extend,
                    crate::routes::lock_clear,
                    crate::routes::upload_nar,
                    crate::routes::finalize_pin,
                ],
            );

        let client = Client::tracked(rocket)
            .await
            .expect("Failed to create test client");

        TestServer {
            client,
            credentials,
            _storage_dir: storage_dir,
            port,
        }
    }
}

impl TestServer {
    /// Create a new test server with default settings
    pub async fn new() -> Self {
        TestServerBuilder::new().build().await
    }

    /// Get the base URL for this test server
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Make a GET request with authentication
    pub fn get_authenticated<'c>(&'c self, uri: &str) -> rocket::local::asynchronous::LocalRequest<'c> {
        self.client
            .get(uri.to_owned())
            .header(rocket::http::Header::new(
                "Authorization",
                format!("Bearer {}", self.credentials.upload_token),
            ))
    }

    /// Make a POST request with authentication
    pub fn post_authenticated<'c>(&'c self, uri: &str) -> rocket::local::asynchronous::LocalRequest<'c> {
        self.client
            .post(uri.to_owned())
            .header(rocket::http::Header::new(
                "Authorization",
                format!("Bearer {}", self.credentials.upload_token),
            ))
    }

    /// Make a PUT request with authentication
    pub fn put_authenticated<'c>(&'c self, uri: &str) -> rocket::local::asynchronous::LocalRequest<'c> {
        self.client
            .put(uri.to_owned())
            .header(rocket::http::Header::new(
                "Authorization",
                format!("Bearer {}", self.credentials.upload_token),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_generation() {
        let creds = TestCredentials::generate();

        // Verify token is non-empty
        assert!(!creds.upload_token.is_empty());

        // Verify hash is valid hex
        assert_eq!(creds.token_hash.len(), 128); // SHA-512 = 64 bytes = 128 hex chars

        // Verify signing key can be parsed
        assert!(creds.signing_key.contains(':'));
        assert!(NixSigningKey::from_config(&creds.signing_key).is_ok());
    }

    #[test]
    fn test_token_hash_verification() {
        let creds = TestCredentials::generate();

        // Manually verify hash
        let mut hasher = Sha512::new();
        hasher.update(creds.upload_token.as_bytes());
        let hash_bytes = hasher.finalize();
        let expected_hash = hex_encode(&hash_bytes);

        assert_eq!(creds.token_hash, expected_hash);
    }
}
