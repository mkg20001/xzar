use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration for xzar client
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Default server name to use when no --server is specified
    pub default_server: Option<String>,
    /// List of configured servers
    #[serde(default)]
    pub servers: Vec<ServerConfig>,
}

/// Configuration for a single server
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server name (used as identifier)
    pub name: String,
    /// Server URL
    pub url: String,
    /// API key for this server
    pub key: String,
}

impl Config {
    /// Get the config file path
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("xzar.toml"))
    }

    /// Load config from the default location
    pub fn load() -> Option<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return None;
        }

        let contents = fs::read_to_string(&path).ok()?;
        toml::from_str(&contents).ok()
    }

    /// Find a server by name or URL
    pub fn find_server(&self, name_or_url: &str) -> Option<&ServerConfig> {
        self.servers
            .iter()
            .find(|s| s.name == name_or_url || s.url == name_or_url)
    }

    /// Get the default server configuration
    pub fn default_server(&self) -> Option<&ServerConfig> {
        let default_name = self.default_server.as_ref()?;
        self.find_server(default_name)
    }

    /// Resolve server URL and key from CLI arguments and config
    ///
    /// Returns (server_url, api_key) or an error message
    pub fn resolve_server_and_key(
        &self,
        server_arg: Option<&str>,
        key_arg: Option<&str>,
    ) -> Result<(String, String), String> {
        match (server_arg, key_arg) {
            // Both provided via CLI - use as-is
            (Some(server), Some(key)) => Ok((server.to_string(), key.to_string())),

            // Only server provided - look up key in config
            (Some(server), None) => {
                if let Some(server_config) = self.find_server(server) {
                    Ok((server_config.url.clone(), server_config.key.clone()))
                } else {
                    Err(format!(
                        "Server '{}' not found in config. Specify --key or add server to {}",
                        server,
                        Self::path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "config file".to_string())
                    ))
                }
            }

            // Only key provided - error (can't determine server from key alone)
            (None, Some(_)) => Err("--server is required when --key is specified".to_string()),

            // Neither provided - use default server from config
            (None, None) => {
                if let Some(server_config) = self.default_server() {
                    Ok((server_config.url.clone(), server_config.key.clone()))
                } else if self.default_server.is_some() {
                    Err(format!(
                        "Default server '{}' not found in config",
                        self.default_server.as_ref().unwrap()
                    ))
                } else {
                    Err(format!(
                        "No --server specified and no default_server in config. \
                        Specify --server and --key, or configure servers in {}",
                        Self::path()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "config file".to_string())
                    ))
                }
            }
        }
    }
}
