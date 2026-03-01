//! xzar-server library
//!
//! This module exposes the core functionality of xzar-server for testing
//! and potential embedding in other applications.

pub mod auth;
pub mod config;
pub mod crypto;
pub mod db;
pub mod error;
pub mod gc;
pub mod models;
pub mod routes;
pub mod schema;
pub mod storage;

/// Test harness module - only compiled for tests or when test_harness feature is enabled
#[cfg(any(test, feature = "test_harness"))]
pub mod test_harness;
