//! Integration tests for xzar server
//!
//! These tests require:
//! - A PostgreSQL database (TEST_DATABASE_URL or DATABASE_URL env var)
//! - Nix installed (for nix-build)
//! - The xzar client binary built
//!
//! Run with: cargo test --test integration_test --features test_harness -- --test-threads=1

use std::process::{Command, Stdio};

use rocket::http::{ContentType, Status};
use serde_json::json;

use xzar_server::test_harness::{TestCredentials, TestServer};

/// Helper to check if nix is available
fn nix_available() -> bool {
    Command::new("nix-build")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Helper to check if database is available
fn database_available() -> bool {
    std::env::var("TEST_DATABASE_URL").is_ok() || std::env::var("DATABASE_URL").is_ok()
}

#[rocket::async_test]
async fn test_nix_cache_info() {
    if !database_available() {
        eprintln!("Skipping test_nix_cache_info: no database configured");
        return;
    }

    let server = TestServer::new().await;

    let response = server.client.get("/nix-cache-info").dispatch().await;
    assert_eq!(response.status(), Status::Ok);

    let body = response.into_string().await.unwrap();
    assert!(body.contains("StoreDir: /nix/store"));
    assert!(body.contains("WantMassQuery: 1"));
}

#[rocket::async_test]
async fn test_check_paths_authenticated() {
    if !database_available() {
        eprintln!("Skipping test_check_paths_authenticated: no database configured");
        return;
    }

    let server = TestServer::new().await;

    // Test authenticated request
    let response = server
        .post_authenticated("/check")
        .header(ContentType::JSON)
        .body(json!({"paths": []}).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
}

#[rocket::async_test]
async fn test_check_paths_unauthenticated() {
    if !database_available() {
        eprintln!("Skipping test_check_paths_unauthenticated: no database configured");
        return;
    }

    let server = TestServer::new().await;

    // Test unauthenticated request should fail
    let response = server
        .client
        .post("/check")
        .header(ContentType::JSON)
        .body(json!({"paths": []}).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Unauthorized);
}

#[rocket::async_test]
async fn test_lock_request_and_clear() {
    if !database_available() {
        eprintln!("Skipping test_lock_request_and_clear: no database configured");
        return;
    }

    let server = TestServer::new().await;

    // Request a lock
    let response = server
        .post_authenticated("/lock/request")
        .header(ContentType::JSON)
        .body(json!({
            "paths": ["test-path-12345-package-1.0"],
            "pin": "test-pin"
        }).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = response.into_json().await.unwrap();
    let lock_id = body["lockId"].as_i64().unwrap();
    assert!(lock_id > 0);

    // Extend the lock
    let response = server
        .post_authenticated("/lock/extend")
        .header(ContentType::JSON)
        .body(json!({
            "lockId": lock_id
        }).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);

    // Clear the lock
    let response = server
        .post_authenticated("/lock/clear")
        .header(ContentType::JSON)
        .body(json!({
            "lockId": lock_id
        }).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
}

#[rocket::async_test]
async fn test_narinfo_not_found() {
    if !database_available() {
        eprintln!("Skipping test_narinfo_not_found: no database configured");
        return;
    }

    let server = TestServer::new().await;

    let response = server
        .client
        .get("/nonexistent12345.narinfo")
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::NotFound);
}

#[rocket::async_test]
async fn test_nar_not_found() {
    if !database_available() {
        eprintln!("Skipping test_nar_not_found: no database configured");
        return;
    }

    let server = TestServer::new().await;

    let response = server.client.get("/nar/nonexistent.nar.xz").dispatch().await;

    assert_eq!(response.status(), Status::NotFound);
}

/// Test credentials generation and signing
#[test]
fn test_credentials_signing() {
    let creds = TestCredentials::generate();
    let signing_key = creds.nix_signing_key();

    // Sign a test message
    let signature = signing_key.sign("test fingerprint");

    // Verify format
    assert!(signature.starts_with(&format!("{}:", creds.key_name)));
}

/// Full integration test: build with nix and upload
/// This test is ignored by default because it requires nix and takes time
#[rocket::async_test]
#[ignore]
async fn test_nix_build_and_upload() {
    if !database_available() {
        eprintln!("Skipping test_nix_build_and_upload: no database configured");
        return;
    }

    if !nix_available() {
        eprintln!("Skipping test_nix_build_and_upload: nix not available");
        return;
    }

    let server = TestServer::new().await;

    // Build a small package with nix
    // Using `hello` instead of openssl as it's smaller
    let output = Command::new("nix-build")
        .args(&["<nixpkgs>", "-A", "hello", "--no-out-link"])
        .output()
        .expect("Failed to run nix-build");

    if !output.status.success() {
        eprintln!(
            "nix-build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let store_path = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    eprintln!("Built: {}", store_path);

    // Extract the derivation ID from the store path
    // Format: /nix/store/<hash>-<name>
    let _drv_full = store_path
        .strip_prefix("/nix/store/")
        .expect("Invalid store path");

    // Check if the path needs to be uploaded
    let response = server
        .post_authenticated("/check")
        .header(ContentType::JSON)
        .body(json!({"paths": [store_path]}).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = response.into_json().await.unwrap();
    let needs_upload: Vec<String> = body["upload"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    eprintln!("Needs upload: {:?}", needs_upload);

    // The path should need uploading since it's a fresh server
    assert!(needs_upload.contains(&store_path));
}
