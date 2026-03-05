use serde::Deserialize;
use std::fs;
use std::path::Path;

use crate::error::{AppError, Result};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub rocket: RocketConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    pub storage: String,
    #[serde(rename = "signingKey")]
    pub signing_key: Option<String>,
    #[serde(rename = "signingPubKey")]
    pub signing_pub_key: Option<String>,
    #[serde(rename = "externalUrl")]
    pub external_url: Option<String>,
    pub db: DbConfig,
    pub sentry_dsn: Option<String>,
    /// OpenID Connect providers configuration
    #[serde(default)]
    pub oidc: Vec<OidcProviderConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RocketConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for RocketConfig {
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
pub struct CorsConfig {
    /// Enable CORS (default: false)
    #[serde(default)]
    pub enabled: bool,
    /// Allowed origins (default: ["*"] if enabled)
    #[serde(default)]
    pub origins: Vec<String>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            origins: Vec::new(),
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

/// Configuration for a single OpenID Connect provider
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OidcProviderConfig {
    /// Unique identifier for this provider (used in URLs and database)
    pub id: String,
    /// Display name for the provider (shown in UI)
    pub name: String,
    /// OpenID Connect issuer URL (e.g., https://accounts.google.com)
    pub issuer_url: String,
    /// OAuth2 client ID
    pub client_id: String,
    /// OAuth2 client secret
    pub client_secret: String,
    /// Whether to use OIDC discovery (default: true)
    /// If false, you must provide authorization_endpoint and token_endpoint
    #[serde(default = "default_true")]
    pub discover: bool,
    /// Authorization endpoint URL (required if discover: false)
    pub authorization_endpoint: Option<String>,
    /// Token endpoint URL (required if discover: false)
    pub token_endpoint: Option<String>,
    /// Userinfo endpoint URL (optional, used for fetching additional claims)
    pub userinfo_endpoint: Option<String>,
    /// JWKS URI for token verification (optional, if not provided tokens won't be signature-verified)
    pub jwks_uri: Option<String>,
    /// OAuth2 scopes to request (default: ["openid", "email", "profile"])
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// Field mapping configuration
    #[serde(default)]
    pub mapping: OidcFieldMapping,
    /// Whether to automatically create users if they don't exist
    #[serde(default)]
    pub auto_create_user: bool,
    /// Whether newly created users should be admins (default: false)
    #[serde(default)]
    pub new_users_admin: bool,
}

fn default_true() -> bool {
    true
}

fn default_scopes() -> Vec<String> {
    vec!["openid".to_string(), "email".to_string(), "profile".to_string()]
}

/// Configuration for mapping OIDC claims to user fields
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OidcFieldMapping {
    /// Claim to use for identifying the user (default: "sub")
    /// This is the unique identifier from the OIDC provider
    #[serde(default = "default_subject_claim")]
    pub subject_claim: String,
    /// Claim to use for the user's name (default: "preferred_username")
    /// Falls back to: name, email (local part), sub
    #[serde(default = "default_name_claim")]
    pub name_claim: String,
    /// Claim to use for the user's email (default: "email")
    #[serde(default = "default_email_claim")]
    pub email_claim: String,
}

fn default_subject_claim() -> String {
    "sub".to_string()
}

fn default_name_claim() -> String {
    "preferred_username".to_string()
}

fn default_email_claim() -> String {
    "email".to_string()
}

impl Default for OidcFieldMapping {
    fn default() -> Self {
        Self {
            subject_claim: default_subject_claim(),
            name_claim: default_name_claim(),
            email_claim: default_email_claim(),
        }
    }
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

        if let Ok(host) = std::env::var("ROCKET_ADDRESS") {
            config.rocket.host = host;
        }

        if let Ok(port) = std::env::var("ROCKET_PORT") {
            if let Ok(p) = port.parse() {
                config.rocket.port = p;
            }
        }

        Ok(config)
    }
}
