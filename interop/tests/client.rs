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
    _storage_dir: TempDir,
    _config_dir: TempDir,
}

impl TestServerProcess {
    /// Start a new test server process
    fn start() -> Self {
        let port = find_available_port();
        let storage_dir = TempDir::new().expect("Failed to create temp storage dir");
        let config_dir = TempDir::new().expect("Failed to create temp config dir");

        // Generate upload token
        let upload_token = format!("test-token-{}", rand::random::<u64>());

        // Generate a signing key (32 bytes, base64 encoded)
        let signing_secret: [u8; 32] = rand::random();
        let signing_key_b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signing_secret,
        );
        let signing_key = format!("test-cache:{}", signing_key_b64);

        // Create config file
        let config_path = config_dir.path().join("config.yaml");
        let database_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("TEST_DATABASE_URL"))
            .expect("DATABASE_URL or TEST_DATABASE_URL must be set");

        let config_content = format!(
            r#"storage: "{}"
db:
  connection: "{}"
tokens:
  - plain: "{}"
signingKey: "{}"
hapi:
  host: "127.0.0.1"
  port: {}
"#,
            storage_dir.path().display(),
            database_url,
            upload_token,
            signing_key,
            port
        );

        std::fs::write(&config_path, &config_content).expect("Failed to write config");

        eprintln!("Starting server on port {} with config:\n{}", port, config_content);

        // Start server process
        // Rocket needs its own env vars for port/address
        let process = Command::new(server_binary())
            .env("XZAR_CONFIG", config_path.to_str().unwrap())
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
            _storage_dir: storage_dir,
            _config_dir: config_dir,
        }
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
}

impl Drop for TestServerProcess {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
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
    assert!(stdout.contains("--pin"));
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
        stderr.contains("--server") || stderr.contains("required"),
        "Should mention missing required arguments"
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
#[ignore] // Requires nix
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

    eprintln!("Built: {}", store_path);

    // Start server
    let mut server = TestServerProcess::start();
    if !server.wait_ready().await {
        panic!("Server failed to start");
    }

    // Run xzar client to upload
    let output = Command::new(client_binary())
        .args([
            "--server",
            &server.url(),
            "--key",
            &server.upload_token,
            "--pin",
            "test-hello",
            &store_path,
        ])
        .output()
        .expect("Failed to run xzar");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    eprintln!("stdout: {}", stdout);
    eprintln!("stderr: {}", stderr);

    assert!(
        output.status.success(),
        "xzar should succeed. stderr: {}",
        stderr
    );
    assert!(
        stdout.contains("Done!") || stdout.contains("Pin"),
        "Should indicate completion"
    );
}

#[tokio::test]
#[ignore] // Requires nix
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
            "--pin",
            "test-pin",
            "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-test",
        ])
        .output()
        .expect("Failed to run xzar");

    // Should fail due to auth error
    assert!(!output.status.success());
}
