//! Common types and utilities for xzar binary cache

use serde::{Deserialize, Serialize};

// ============ Duration Formatting ============

/// Format milliseconds as human-readable duration
pub fn format_duration(ms: i64) -> String {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const WEEK_MS: i64 = 7 * DAY_MS;
    const MONTH_MS: i64 = 30 * DAY_MS;
    const YEAR_MS: i64 = 365 * DAY_MS;

    if ms >= YEAR_MS {
        let years = ms / YEAR_MS;
        format!("{}y", years)
    } else if ms >= MONTH_MS {
        let months = ms / MONTH_MS;
        format!("{}m", months)
    } else if ms >= WEEK_MS {
        let weeks = ms / WEEK_MS;
        format!("{}w", weeks)
    } else if ms >= DAY_MS {
        let days = ms / DAY_MS;
        format!("{}d", days)
    } else {
        format!("{}ms", ms)
    }
}

// ============ API Types ============

/// Root path in a pin
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinRoot {
    pub drv_id: String,
    pub drv_full: String,
}

/// Pin with its roots (API response)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinResponse {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created: String,
    pub expires: Option<String>,
    pub abandoned: bool,
    pub leave_after_abandon: Option<i64>,
    pub roots: Vec<PinRoot>,
}

/// Request to check which paths are not in the cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckRequest {
    pub paths: Vec<String>,
}

/// Response with paths that need to be uploaded
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResponse {
    pub need: Vec<String>,
}

/// Request to finalize a pin
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalizePinRequest {
    pub roots: Vec<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leave_after_abandon: Option<u64>,
}

/// Lock response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockResponse {
    pub lock: i32,
    pub deadline: String,
}

/// Request to extend or clear a lock
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockRequest {
    pub lock: i32,
}

// ============ Self/Auth Info Types ============

/// Response from /self endpoint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfResponse {
    pub is_admin: bool,
    pub credential_type: String,
    pub user: Option<UserInfo>,
}

/// User info in self response
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: i32,
    pub name: String,
}

// ============ Admin API Types ============

/// User response from admin API
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserResponse {
    pub id: i32,
    pub name: String,
    pub email: Option<String>,
    pub is_admin: bool,
    pub created: String,
}

/// Token response from admin API
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminTokenResponse {
    pub id: i32,
    pub user_id: Option<i32>,
    pub user_name: Option<String>,
    pub is_system: bool,
    pub description: Option<String>,
    pub created: String,
}

/// Request to create a user
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub name: String,
    #[serde(default)]
    pub is_admin: bool,
}

/// Request to update a user (PATCH style - only specified fields are updated)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<Option<String>>,
}

/// Request to create a token
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTokenRequest {
    pub user_id: Option<i32>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Response after creating a token (includes raw token)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTokenResponse {
    pub id: i32,
    pub token: String,
}

/// Request to update a token
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTokenRequest {
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(86400000), "1d");
        assert_eq!(format_duration(86400000 * 7), "1w");
        assert_eq!(format_duration(86400000 * 30), "1m");
        assert_eq!(format_duration(86400000 * 365), "1y");
        assert_eq!(format_duration(86400000 * 14), "2w");
    }
}
