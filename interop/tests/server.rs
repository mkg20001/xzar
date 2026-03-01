//! Server API integration tests
//!
//! These tests verify the xzar server API using Rocket's test client.
//!
//! Requirements:
//! - PostgreSQL database (TEST_DATABASE_URL or DATABASE_URL env var)
//!
//! Run with: cargo test -p xzar-interop --test server -- --test-threads=1

use rocket::http::{ContentType, Status};
use serde_json::json;

use xzar_server::test_harness::{TestCredentials, TestServer};

/// Helper to check if database is available
fn database_available() -> bool {
    std::env::var("TEST_DATABASE_URL").is_ok() || std::env::var("DATABASE_URL").is_ok()
}

#[rocket::async_test]
async fn test_nix_cache_info() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
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
        eprintln!("Skipping test: no database configured");
        return;
    }

    let server = TestServer::new().await;

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
        eprintln!("Skipping test: no database configured");
        return;
    }

    let server = TestServer::new().await;

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
        eprintln!("Skipping test: no database configured");
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
    let lock_id = body["lock"].as_i64().unwrap();
    assert!(lock_id > 0);

    // Extend the lock
    let response = server
        .post_authenticated("/lock/extend")
        .header(ContentType::JSON)
        .body(json!({
            "lock": lock_id
        }).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);

    // Clear the lock
    let response = server
        .post_authenticated("/lock/clear")
        .header(ContentType::JSON)
        .body(json!({
            "lock": lock_id
        }).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
}

#[rocket::async_test]
async fn test_narinfo_not_found() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
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
        eprintln!("Skipping test: no database configured");
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

#[rocket::async_test]
async fn test_check_multiple_paths() {
    if !database_available() {
        eprintln!("Skipping test: no database configured");
        return;
    }

    let server = TestServer::new().await;

    let paths = vec![
        "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-pkg1",
        "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-pkg2",
        "/nix/store/cccccccccccccccccccccccccccccccc-pkg3",
    ];

    let response = server
        .post_authenticated("/check")
        .header(ContentType::JSON)
        .body(json!({"paths": paths}).to_string())
        .dispatch()
        .await;

    assert_eq!(response.status(), Status::Ok);
    let body: serde_json::Value = response.into_json().await.unwrap();

    // All paths should need upload since they don't exist
    let need = body["need"].as_array().unwrap();
    assert_eq!(need.len(), 3);
}
