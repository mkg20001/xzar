use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::error::{AppError, Result};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub hapi: HapiConfig,
    pub storage: String,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(rename = "signingKey")]
    pub signing_key: Option<String>,
    #[serde(rename = "signingPubKey")]
    pub signing_pub_key: Option<String>,
    #[serde(rename = "externalUrl")]
    pub external_url: Option<String>,
    pub db: DbConfig,
    pub sentry_dsn: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HapiConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for HapiConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

fn default_host() -> String {
    "::".to_string()
}

fn default_port() -> u16 {
    17788
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum TokenConfig {
    Hashed { hashed: String },
    Plain { plain: String },
}

impl TokenConfig {
    pub fn get_hash(&self) -> String {
        use sha2::{Digest, Sha512};

        match self {
            TokenConfig::Hashed { hashed } => hashed.clone(),
            TokenConfig::Plain { plain } => {
                let mut hasher = Sha512::new();
                hasher.update(plain.as_bytes());
                let result = hasher.finalize();
                hex::encode(result)
            }
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DbConfig {
    #[serde(default = "default_client")]
    pub client: String,
    pub connection: String,
}

fn default_client() -> String {
    "pg".to_string()
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|e| AppError::Config(format!("Failed to read config file: {}", e)))?;

        let mut config: Config = serde_yaml::from_str(&contents)
            .map_err(|e| AppError::Config(format!("Failed to parse config file: {}", e)))?;

        // Override with environment variables if present
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            config.db.connection = db_url;
        }

        if let Ok(storage) = std::env::var("XZAR_STORAGE") {
            config.storage = storage;
        }

        if let Ok(signing_key) = std::env::var("XZAR_SIGNING_KEY") {
            config.signing_key = Some(signing_key);
        }

        if let Ok(signing_pub_key) = std::env::var("XZAR_SIGNING_PUB_KEY") {
            config.signing_pub_key = Some(signing_pub_key);
        }

        if let Ok(external_url) = std::env::var("XZAR_EXTERNAL_URL") {
            config.external_url = Some(external_url);
        }

        if let Ok(sentry_dsn) = std::env::var("SENTRY_DSN") {
            config.sentry_dsn = Some(sentry_dsn);
        }

        if let Ok(host) = std::env::var("XZAR_HOST") {
            config.hapi.host = host;
        }

        if let Ok(port) = std::env::var("XZAR_PORT") {
            if let Ok(p) = port.parse() {
                config.hapi.port = p;
            }
        }

        Ok(config)
    }

    pub fn get_token_hashes(&self) -> Vec<String> {
        self.tokens.iter().map(|t| t.get_hash()).collect()
    }
}

// Add hex encoding for SHA512 hashes
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        let bytes = bytes.as_ref();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for &byte in bytes {
            hex.push(HEX_CHARS[(byte >> 4) as usize] as char);
            hex.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
        }
        hex
    }
}
