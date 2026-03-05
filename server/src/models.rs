use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{drv_locks, drv_pins, drvs, locks, oidc_identities, oidc_sessions, pins, tokens, users};

// ============ Derivations ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = drvs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Drv {
    pub drv_id: String,
    pub drv_full: String,
    pub nar_hash: String,
    pub nar_size: i64,
    pub file_hash: String,
    pub file_size: i64,
    pub deriver: Option<String>,
    pub sig: Option<String>,
    pub refs: Vec<Option<String>>,
    pub nar_comp: Option<String>,
    pub nar_file: String,
    pub nar_file_storage: String,
    pub gc: bool,
    pub created: NaiveDateTime,
    pub last_fetched: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = drvs)]
pub struct NewDrv {
    pub drv_id: String,
    pub drv_full: String,
    pub nar_hash: String,
    pub nar_size: i64,
    pub file_hash: String,
    pub file_size: i64,
    pub deriver: Option<String>,
    pub sig: Option<String>,
    pub refs: Vec<Option<String>>,
    pub nar_comp: Option<String>,
    pub nar_file: String,
    pub nar_file_storage: String,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = drvs)]
pub struct UpdateDrv {
    pub last_fetched: Option<NaiveDateTime>,
    pub gc: Option<bool>,
}

// ============ Locks ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = locks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Lock {
    pub id: i32,
    pub expires: NaiveDateTime,
    pub owner: i32,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = locks)]
pub struct NewLock {
    pub owner: i32,
}

// ============ Pins ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = pins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pin {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created: NaiveDateTime,
    pub expires: Option<NaiveDateTime>,
    pub abandoned: bool,
    pub leave_after_abandon: Option<i64>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = pins)]
pub struct NewPin {
    pub name: String,
    pub description: Option<String>,
    pub expires: Option<NaiveDateTime>,
    pub leave_after_abandon: Option<i64>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = pins)]
pub struct UpdatePin {
    pub abandoned: Option<bool>,
    pub expires: Option<Option<NaiveDateTime>>,
}

// ============ Junction Tables ============

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = drv_locks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DrvLock {
    pub drv_id: String,
    pub lock_id: i32,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = drv_pins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DrvPin {
    pub drv_id: String,
    pub pin_id: i32,
}

// ============ Users ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i32,
    pub name: String,
    pub is_admin: bool,
    pub created: NaiveDateTime,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub name: String,
    pub is_admin: bool,
}

// ============ Tokens ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = tokens)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Token {
    pub id: i32,
    pub user_id: Option<i32>,
    pub token_hash: String,
    pub is_system: bool,
    pub description: Option<String>,
    pub created: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = tokens)]
pub struct NewToken {
    pub user_id: Option<i32>,
    pub token_hash: String,
    pub is_system: bool,
    pub description: Option<String>,
}

// ============ OIDC Identities ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = oidc_identities)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OidcIdentity {
    pub id: i32,
    pub provider_id: String,
    pub subject: String,
    pub user_id: Option<i32>,
    pub cached_email: Option<String>,
    pub cached_name: Option<String>,
    pub created: NaiveDateTime,
    pub last_login: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = oidc_identities)]
pub struct NewOidcIdentity {
    pub provider_id: String,
    pub subject: String,
    pub user_id: Option<i32>,
    pub cached_email: Option<String>,
    pub cached_name: Option<String>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = oidc_identities)]
pub struct UpdateOidcIdentity {
    pub user_id: Option<i32>,
    pub cached_email: Option<Option<String>>,
    pub cached_name: Option<Option<String>>,
    pub last_login: Option<NaiveDateTime>,
}

// ============ OIDC Sessions ============

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = oidc_sessions)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OidcSession {
    pub id: i32,
    pub state: String,
    pub provider_id: String,
    pub nonce: String,
    pub redirect_url: Option<String>,
    pub expires: NaiveDateTime,
    pub created: NaiveDateTime,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = oidc_sessions)]
pub struct NewOidcSession {
    pub state: String,
    pub provider_id: String,
    pub nonce: String,
    pub redirect_url: Option<String>,
    pub expires: NaiveDateTime,
}

// ============ Auth Result Types ============

/// Result of token validation - represents the authenticated entity
#[derive(Debug, Clone)]
pub enum AuthenticatedEntity {
    /// System token (admin-level, no associated user)
    System { token_id: i32 },
    /// User token with associated user info
    User { token_id: i32, user: User },
}

impl AuthenticatedEntity {
    pub fn is_admin(&self) -> bool {
        match self {
            AuthenticatedEntity::System { .. } => true,
            AuthenticatedEntity::User { user, .. } => user.is_admin,
        }
    }

    pub fn user(&self) -> Option<&User> {
        match self {
            AuthenticatedEntity::System { .. } => None,
            AuthenticatedEntity::User { user, .. } => Some(user),
        }
    }

    pub fn token_id(&self) -> i32 {
        match self {
            AuthenticatedEntity::System { token_id } => *token_id,
            AuthenticatedEntity::User { token_id, .. } => *token_id,
        }
    }
}

// ============ Request/Response DTOs ============

#[derive(Debug, Deserialize)]
pub struct CheckRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CheckResponse {
    pub need: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LockResponse {
    pub lock: i32,
    pub deadline: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct LockExtendRequest {
    pub lock: i32,
}

#[derive(Debug, Deserialize)]
pub struct LockClearRequest {
    pub lock: i32,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Deserialize)]
pub struct FinalizePinRequest {
    pub roots: Vec<String>,
    pub name: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(rename = "leaveAfterAbandon", default)]
    pub leave_after_abandon: Option<i64>,
}

// ============ Pins List Response ============

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinWithRoots {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created: NaiveDateTime,
    pub expires: Option<NaiveDateTime>,
    pub abandoned: bool,
    pub leave_after_abandon: Option<i64>,
    pub roots: Vec<PinRoot>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinRoot {
    pub drv_id: String,
    pub drv_full: String,
}

// Re-export admin API types from common
pub use xzar_common::{
    AdminTokenResponse, AdminUserResponse, CreateTokenRequest, CreateTokenResponse,
    CreateUserRequest, SelfResponse, UpdateTokenRequest, UpdateUserRequest, UserInfo,
};
