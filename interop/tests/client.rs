//! Client CLI integration tests
//!
//! These tests verify the xzar client CLI by spawning it as a subprocess
//! and testing it against a real xzar server instance.
//!
//! Requirements:
//! - PostgreSQL database (TEST_DATABASE_URL or DATABASE_URL env var)
//! - Nix installed (for nix-store operations)
//! - xzar client binary built (cargo build -p xzar-client)
//!
//! Run with: cargo test -p xzar-interop --test client -- --test-threads=1

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use tempfile::TempDir;
use tokio::time::sleep;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(19080);

/// Helper to check if database is available
fn database_available() -> bool {
    std::env::var("TEST_DATABASE_URL").is_ok() || std::env::var("DATABASE_URL").is_ok()
}

/// Helper to check if nix is available
fn nix_available() -> bool {
    Command::new("nix-store")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get path to the xzar client binary
fn client_binary() -> PathBuf {
    // Try release build first, then debug
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/release/xzar");

    if release.exists() {
        return release;
    }

    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug/xzar");

    if debug.exists() {
        return debug;
    }

    panic!(
        "xzar client binary not found. Run 'cargo build -p xzar-client' first.\n\
         Looked in:\n  - {}\n  - {}",
        release.display(),
        debug.display()
    );
}

/// Get path to the xzar server binary
fn server_binary() -> PathBuf {
    let release = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/release/xzar-server");

    if release.exists() {
        return release;
    }

    let debug = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/debug/xzar-server");

    if debug.exists() {
        return debug;
    }

    panic!(
        "xzar-server binary not found. Run 'cargo build -p xzar-server' first.\n\
         Looked in:\n  - {}\n  - {}",
        release.display(),
        debug.display()
    );
}

/// Find an available port
fn find_available_port() -> u16 {
    let start_port = PORT_COUNTER.fetch_add(10, Ordering::SeqCst);
    for port in start_port..start_port + 10 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("Could not find available port");
}

/// Test server configuration
struct TestServerProcess {
    process: std::process::Child,
    port: u16,
    upload_token: String,
    signing_secret: [u8; 32],
    key_name: String,
    database_name: String,
    base_database_url: String,
    config_path: PathBuf,
    _storage_dir: TempDir,
    _config_dir: TempDir,
}

impl TestServerProcess {
    /// Start a new test server process
    fn start() -> Self {
        let port = find_available_port();
        let storage_dir = TempDir::new().expect("Failed to create temp storage dir");
        let config_dir = TempDir::new().expect("Failed to create temp config dir");

        // Get the base database URL and create a unique database for this test
        let base_database_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("TEST_DATABASE_URL"))
            .expect("DATABASE_URL or TEST_DATABASE_URL must be set");

        // Generate unique database name for this test
        let database_name = format!("xzar_test_{}", port);

        // Drop database if it exists from a previous failed run, then create fresh
        let drop_result = Command::new("psql")
            .args([&base_database_url, "-c", &format!("DROP DATABASE IF EXISTS {}", database_name)])
            .output();
        if let Ok(output) = drop_result {
            if !output.status.success() {
                eprintln!("Warning: failed to drop database: {}", String::from_utf8_lossy(&output.stderr));
            }
        }

        let create_result = Command::new("psql")
            .args([&base_database_url, "-c", &format!("CREATE DATABASE {}", database_name)])
            .output()
            .expect("Failed to run psql to create database");

        if !create_result.status.success() {
            panic!(
                "Failed to create database {}: {}",
                database_name,
                String::from_utf8_lossy(&create_result.stderr)
            );
        }
        eprintln!("Created test database: {}", database_name);

        // Build database URL for the new database
        // The base URL format is like: postgres://?host=/tmp/socket&dbname=xzar_test
        // We need to replace dbname with our new database name
        let database_url = if let Some(idx) = base_database_url.find("dbname=") {
            // Find the end of the dbname value (either & or end of string)
            let after_dbname = &base_database_url[idx + 7..];
            let end_idx = after_dbname.find('&').unwrap_or(after_dbname.len());
            format!(
                "{}dbname={}{}",
                &base_database_url[..idx],
                database_name,
                &after_dbname[end_idx..]
            )
        } else {
            // Append dbname
            format!("{}&dbname={}", base_database_url, database_name)
        };
        eprintln!("Using database URL: {}", database_url);

        // Generate a signing key in Nix format (64 bytes: seed + public key)
        // Nix/libsodium Ed25519 secret key format is [32-byte seed][32-byte public key]
        let signing_seed: [u8; 32] = rand::random();
        // Use cache name matching the server - nix expects key name to identify the cache
        // Note: key name cannot contain colons as the format is "name:base64key"
        let key_name = format!("cache.localhost-{}", port);

        // Derive public key from seed using ed25519_dalek
        use ed25519_dalek::SigningKey;
        let dalek_key = SigningKey::from_bytes(&signing_seed);
        let public_key_bytes = dalek_key.verifying_key().to_bytes();

        // Concatenate seed + public key to form 64-byte Nix secret key
        let mut nix_secret_key = [0u8; 64];
        nix_secret_key[..32].copy_from_slice(&signing_seed);
        nix_secret_key[32..].copy_from_slice(&public_key_bytes);

        let signing_key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            nix_secret_key,
        );
        let signing_key = format!("{}:{}", key_name, signing_key_b64);

        // Create config file (tokens are now in database, not config)
        let config_path = config_dir.path().join("config.yaml");

        let config_content = format!(
            r#"storage: "{}"
db:
  connection: "{}"
signingKey: "{}"
rocket:
  host: "127.0.0.1"
  port: {}
"#,
            storage_dir.path().display(),
            database_url,
            signing_key,
            port
        );

        std::fs::write(&config_path, &config_content).expect("Failed to write config");

        eprintln!("Starting server on port {} with config:\n{}", port, config_content);

        // Create a system token in the database using the CLI
        // IMPORTANT: Remove DATABASE_URL to prevent it from overriding the config file
        let token_output = Command::new(server_binary())
            .args(["token", "create", "--system", "--description", "Integration test token"])
            .env("XZAR_CONFIG", config_path.to_str().unwrap())
            .env_remove("DATABASE_URL")
            .env_remove("TEST_DATABASE_URL")
            .output()
            .expect("Failed to create test token");

        if !token_output.status.success() {
            panic!(
                "Failed to create token: {}",
                String::from_utf8_lossy(&token_output.stderr)
            );
        }

        // Parse the token from output (format: "Token: <token>")
        let token_stdout = String::from_utf8_lossy(&token_output.stdout);
        let upload_token = token_stdout
            .lines()
            .find(|line| line.starts_with("Token: "))
            .map(|line| line.strip_prefix("Token: ").unwrap().trim().to_string())
            .expect("Could not find token in CLI output");

        eprintln!("Created test token: {}", upload_token);

        // Start server process
        // Rocket needs its own env vars for port/address
        // IMPORTANT: Remove DATABASE_URL to prevent it from overriding the config file
        let process = Command::new(server_binary())
            .args(["serve"])
            .env("XZAR_CONFIG", config_path.to_str().unwrap())
            .env_remove("DATABASE_URL")
            .env_remove("TEST_DATABASE_URL")
            .env("ROCKET_ADDRESS", "127.0.0.1")
            .env("ROCKET_PORT", port.to_string())
            .env("ROCKET_LOG_LEVEL", "off")
            .env("RUST_LOG", "warn")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start xzar-server");

        Self {
            process,
            port,
            upload_token,
            signing_secret: signing_seed,
            key_name,
            database_name,
            base_database_url,
            config_path,
            _storage_dir: storage_dir,
            _config_dir: config_dir,
        }
    }

    /// Get the public key in Nix format (keyname:base64pubkey)
    /// Used for trusted-public-keys configuration
    fn public_key(&self) -> String {
        use ed25519_dalek::SigningKey;
        let signing_key = SigningKey::from_bytes(&self.signing_secret);
        let verifying_key = signing_key.verifying_key();
        let pub_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            verifying_key.as_bytes(),
        );
        format!("{}:{}", self.key_name, pub_b64)
    }

    /// Wait for server to be ready
    async fn wait_ready(&mut self) -> bool {
        let url = format!("http://127.0.0.1:{}/nix-cache-info", self.port);

        for i in 0..50 {
            // Check if process has exited
            if let Ok(Some(status)) = self.process.try_wait() {
                eprintln!("Server process exited with status: {:?}", status);
                // Try to read stderr
                if let Some(ref mut stderr) = self.process.stderr {
                    use std::io::Read;
                    let mut output = String::new();
                    let _ = stderr.read_to_string(&mut output);
                    eprintln!("Server stderr: {}", output);
                }
                return false;
            }

            if let Ok(response) = reqwest::get(&url).await {
                if response.status().is_success() {
                    eprintln!("Server ready after {} attempts", i + 1);
                    return true;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        eprintln!("Server failed to become ready after 50 attempts");
        false
    }

    /// Get the server URL
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Run garbage collection via CLI command
    async fn run_gc(&self) -> Result<(), String> {
        let output = Command::new(server_binary())
            .args(["gc"])
            .env("XZAR_CONFIG", self.config_path.to_str().unwrap())
            .env_remove("DATABASE_URL")
            .env_remove("TEST_DATABASE_URL")
            .output()
            .expect("Failed to run xzar-server gc");

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "GC failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}

impl Drop for TestServerProcess {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();

        // Drop the test database
        let drop_result = Command::new("psql")
            .args([&self.base_database_url, "-c", &format!("DROP DATABASE IF EXISTS {}", self.database_name)])
            .output();
        if let Ok(output) = drop_result {
            if output.status.success() {
                eprintln!("Dropped test database: {}", self.database_name);
            } else {
                eprintln!("Warning: failed to drop database {}: {}", self.database_name, String::from_utf8_lossy(&output.stderr));
            }
        }
    }
}

#[test]
fn test_client_binary_exists() {
    let binary = client_binary();
    assert!(binary.exists(), "Client binary should exist at {:?}", binary);
}

#[test]
fn test_client_help() {
    let output = Command::new(client_binary())
        .arg("--help")
        .output()
        .expect("Failed to run xzar --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("xzar"));
    assert!(stdout.contains("--server"));
    assert!(stdout.contains("--key"));
    assert!(stdout.contains("upload"));
    assert!(stdout.contains("list"));
}

#[test]
fn test_client_version() {
    let output = Command::new(client_binary())
        .arg("--version")
        .output()
        .expect("Failed to run xzar --version");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("xzar"));
}

#[test]
fn test_client_missing_required_args() {
    let output = Command::new(client_binary())
        .output()
        .expect("Failed to run xzar");

    // Should fail without required arguments
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--server") || stderr.contains("required") || stderr.contains("Usage"),
        "Should mention missing required arguments or show usage"
    );
}

#[tokio::test]
async fn test_client_no_paths_error() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // Run client without any paths - should error
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-pin",
        ])
        .output()
        .expect("Failed to run xzar");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No paths") || stderr.contains("paths"),
        "Should mention missing paths"
    );
}

#[tokio::test]
async fn test_client_upload_hello() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    if !nix_available() {
        eprintln!("Skipping test: nix not available");
        return;
    }

    // Build hello package
    let build_output = Command::new("nix-build")
        .args(["<nixpkgs>", "-A", "hello", "--no-out-link"])
        .output()
        .expect("Failed to run nix-build");

    if !build_output.status.success() {
        panic!(
            "nix-build failed: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
    }

    let store_path = String::from_utf8_lossy(&build_output.stdout)
        .trim()
        .to_string();

    // Start server
    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // Run xzar client to upload with leave-after-abandon
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-hello",
            "--leave-after-abandon",
            "7d",
            &store_path,
        ])
        .output()
        .expect("Failed to run xzar");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "xzar upload failed: {}", stderr);

    // Verify leave-after-abandon is stored by listing pins
    let list_output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "list",
        ])
        .output()
        .expect("Failed to run xzar list");

    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(
        list_stdout.contains("Leave after abandon: 1w"),
        "Should show leave after abandon duration. Got: {}",
        list_stdout
    );
}

#[tokio::test]
async fn test_client_stdin_paths() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    if !nix_available() {
        eprintln!("Skipping test: nix not available");
        return;
    }

    // Build hello package
    let build_output = Command::new("nix-build")
        .args(["<nixpkgs>", "-A", "hello", "--no-out-link"])
        .output()
        .expect("Failed to run nix-build");

    if !build_output.status.success() {
        panic!(
            "nix-build failed: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
    }

    let store_path = String::from_utf8_lossy(&build_output.stdout)
        .trim()
        .to_string();

    // Start server
    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // Run xzar client with paths via stdin
    let mut child = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn xzar");

    // Write path to stdin
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("Failed to get stdin");
        writeln!(stdin, "{}", store_path).expect("Failed to write to stdin");
    }

    let output = child.wait_with_output().expect("Failed to wait for xzar");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    eprintln!("stdout: {}", stdout);
    eprintln!("stderr: {}", stderr);

    assert!(
        output.status.success(),
        "xzar should succeed. stderr: {}",
        stderr
    );
}

#[tokio::test]
async fn test_client_invalid_server() {
    // Try to connect to a server that doesn't exist
    let output = Command::new(client_binary())
        .args([
            "--server",
            "http://127.0.0.1:1",
            "--key",
            "test-key",
            "upload",
            "--pin",
            "test-pin",
            "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-test",
        ])
        .output()
        .expect("Failed to run xzar");

    // Should fail to connect
    assert!(!output.status.success());
}

#[tokio::test]
async fn test_client_invalid_auth() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // Try with wrong token
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            "wrong-token",
            "upload",
            "--pin",
            "test-pin",
            "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-test",
        ])
        .output()
        .expect("Failed to run xzar");

    // Should fail due to auth error
    assert!(!output.status.success());
}

/// Test uploading a package and fetching it with nix-store --realise
///
/// This test:
/// 1. Builds a simple nix package (hello)
/// 2. Uploads it to the xzar server using the client
/// 3. Creates a temporary Nix store
/// 4. Uses nix-store --realise to fetch the package from the server
/// 5. Verifies the package was downloaded correctly
#[tokio::test]
async fn test_nix_store_realise_from_cache() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    if !nix_available() {
        eprintln!("Skipping test: nix not available");
        return;
    }

    // Build hello package
    let build_output = Command::new("nix-build")
        .args(["<nixpkgs>", "-A", "hello", "--no-out-link"])
        .output()
        .expect("Failed to run nix-build");

    if !build_output.status.success() {
        panic!(
            "nix-build failed: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
    }

    let store_path = String::from_utf8_lossy(&build_output.stdout)
        .trim()
        .to_string();

    eprintln!("Built: {}", store_path);

    // Start server
    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    let public_key = server.public_key();
    eprintln!("Server public key: {}", public_key);

    // Upload the package using xzar client
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-realise",
            &store_path,
        ])
        .output()
        .expect("Failed to run xzar");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("Upload stdout: {}", stdout);
    eprintln!("Upload stderr: {}", stderr);

    if !output.status.success() {
        panic!("xzar upload failed: {}", stderr);
    }

    // Verify narinfo has a signature
    let drv_id = store_path
        .strip_prefix("/nix/store/")
        .unwrap()
        .split('-')
        .next()
        .unwrap();
    let narinfo_url = format!("{}/{}.narinfo", server.url(), drv_id);
    let narinfo_response = reqwest::get(&narinfo_url).await.expect("Failed to fetch narinfo");
    let narinfo_text = narinfo_response.text().await.expect("Failed to read narinfo");

    assert!(
        narinfo_text.contains("Sig:"),
        "Narinfo should contain a Sig field"
    );

    // Create a temporary Nix store for testing
    let temp_store = TempDir::new().expect("Failed to create temp store dir");
    let store_root = temp_store.path();
    let nix_store_path = store_root.join("nix/store");
    std::fs::create_dir_all(&nix_store_path).expect("Failed to create nix/store dir");

    let path_name = store_path
        .strip_prefix("/nix/store/")
        .expect("Invalid store path");

    // Use nix copy to fetch from the cache to a local store
    let store_url = format!("local?root={}", store_root.display());
    let substituters = format!("http://cache.localhost:{}", server.port);

    // Configure Nix to use our cache with signature verification
    let nix_config = format!(
        "substituters = {}\ntrusted-substituters = {}\ntrusted-public-keys = {}\nrequire-sigs = true\nnarinfo-cache-positive-ttl = 0\nnarinfo-cache-negative-ttl = 0\n",
        substituters, substituters, public_key
    );

    let copy_output = Command::new("nix")
        .args([
            "copy",
            "--from",
            &substituters,
            "--to",
            &store_url,
            &store_path,
        ])
        .env("NIX_CONFIG", &nix_config)
        .output()
        .expect("Failed to run nix copy");

    let copy_stderr = String::from_utf8_lossy(&copy_output.stderr);
    assert!(
        copy_output.status.success(),
        "nix copy failed: {}",
        copy_stderr
    );

    // Verify the path exists in our temp store
    let fetched_path = nix_store_path.join(path_name);
    assert!(
        fetched_path.exists(),
        "Fetched path {} does not exist",
        fetched_path.display()
    );

    // Verify it's a directory (hello package should have bin/, etc.)
    assert!(
        fetched_path.is_dir(),
        "Fetched path {} is not a directory",
        fetched_path.display()
    );

    // Check for the hello binary
    let hello_bin = fetched_path.join("bin/hello");
    assert!(
        hello_bin.exists(),
        "hello binary {} does not exist",
        hello_bin.display()
    );

    eprintln!("Successfully fetched {} from cache!", path_name);
}

/// Test that GC respects leave_after_abandon when pins are replaced
#[tokio::test]
async fn test_gc_respects_leave_after_abandon() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    if !nix_available() {
        eprintln!("Skipping test: nix not available");
        return;
    }

    // Build hello package
    let build_output = Command::new("nix-build")
        .args(["<nixpkgs>", "-A", "hello", "--no-out-link"])
        .output()
        .expect("Failed to run nix-build");

    if !build_output.status.success() {
        panic!(
            "nix-build failed: {}",
            String::from_utf8_lossy(&build_output.stderr)
        );
    }

    let store_path = String::from_utf8_lossy(&build_output.stdout)
        .trim()
        .to_string();

    // Start server
    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // First upload with leave-after-abandon
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-gc-pin",
            "--leave-after-abandon",
            "7d",
            &store_path,
        ])
        .output()
        .expect("Failed to run xzar");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "First upload failed: {}", stderr);

    // Second upload with same pin name (should abandon the first)
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "upload",
            "--pin",
            "test-gc-pin",
            &store_path,
        ])
        .output()
        .expect("Failed to run xzar");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "Second upload failed: {}", stderr);

    // Run GC via the test harness
    server.run_gc().await.expect("GC failed");

    // Verify the narinfo is still accessible (data not GC'd because leave_after_abandon)
    let drv_id = store_path
        .strip_prefix("/nix/store/")
        .unwrap()
        .split('-')
        .next()
        .unwrap();
    let narinfo_url = format!("{}/{}.narinfo", server.url(), drv_id);
    let response = reqwest::get(&narinfo_url).await.expect("Failed to fetch narinfo");

    assert!(
        response.status().is_success(),
        "Narinfo was GC'd but should have been kept due to leave_after_abandon"
    );
}
