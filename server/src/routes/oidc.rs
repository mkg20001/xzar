//! OpenID Connect authentication routes
//!
//! Provides routes for OIDC login flow:
//! - `GET /auth/oidc/providers` - List available OIDC providers
//! - `GET /auth/oidc/<provider>/login` - Initiate OIDC login
//! - `GET /auth/oidc/<provider>/callback` - Handle OIDC callback

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel::PgConnection;
use openid::{Client, Discovered, Options, StandardClaims, Token, Userinfo};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::Redirect;
use rocket::serde::json::Json;
use rocket::{get, State};
use serde::Serialize;
use url::Url;

use crate::auth::hash_token;
use crate::config::{Config, OidcProviderConfig};
use crate::db::Database;
use crate::models::{NewOidcIdentity, NewOidcSession, NewToken, NewUser, OidcIdentity, OidcSession, User};
use crate::schema::{oidc_identities, oidc_sessions, tokens, users};

/// Initialized OIDC clients for each provider
pub struct OidcClients {
    clients: HashMap<String, OidcClientInfo>,
}

struct OidcClientInfo {
    client: Client<Discovered, StandardClaims>,
    config: OidcProviderConfig,
}

impl OidcClients {
    /// Initialize OIDC clients from configuration
    pub async fn from_config(config: &Config) -> std::result::Result<Self, String> {
        let mut clients = HashMap::new();

        for provider_config in &config.oidc {
            tracing::info!("Initializing OIDC provider: {}", provider_config.id);

            // Build redirect URL
            let redirect_url = format!(
                "{}/auth/oidc/{}/callback",
                config
                    .external_url
                    .as_ref()
                    .map(|s| s.trim_end_matches('/'))
                    .unwrap_or("http://localhost:17788"),
                provider_config.id
            );

            let issuer = Url::parse(&provider_config.issuer_url)
                .map_err(|e| format!("Invalid issuer URL for {}: {}", provider_config.id, e))?;

            // Discover and create client
            let client = Client::<Discovered, StandardClaims>::discover(
                provider_config.client_id.clone(),
                provider_config.client_secret.clone(),
                Some(redirect_url),
                issuer,
            )
            .await
            .map_err(|e| {
                format!(
                    "Failed to discover OIDC metadata for {}: {}",
                    provider_config.id, e
                )
            })?;

            clients.insert(
                provider_config.id.clone(),
                OidcClientInfo {
                    client,
                    config: provider_config.clone(),
                },
            );

            tracing::info!("OIDC provider {} initialized successfully", provider_config.id);
        }

        Ok(Self { clients })
    }

    fn get(&self, provider_id: &str) -> Option<&OidcClientInfo> {
        self.clients.get(provider_id)
    }

    pub fn providers(&self) -> Vec<ProviderInfo> {
        self.clients
            .values()
            .map(|info| ProviderInfo {
                id: info.config.id.clone(),
                name: info.config.name.clone(),
            })
            .collect()
    }
}

/// Provider info for listing
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
}

/// List available OIDC providers
#[get("/auth/oidc/providers")]
pub fn list_providers(oidc: &State<Arc<OidcClients>>) -> Json<Vec<ProviderInfo>> {
    Json(oidc.providers())
}

/// Initiate OIDC login flow
#[get("/auth/oidc/<provider_id>/login?<redirect>")]
pub async fn oidc_login(
    provider_id: &str,
    redirect: Option<String>,
    database: &State<Database>,
    oidc: &State<Arc<OidcClients>>,
) -> std::result::Result<Redirect, Status> {
    let client_info = match oidc.get(provider_id) {
        Some(info) => info,
        None => {
            tracing::warn!("Unknown OIDC provider: {}", provider_id);
            return Err(Status::NotFound);
        }
    };

    // Build options with scopes
    let options = Options {
        scope: Some(client_info.config.scopes.join(" ")),
        ..Default::default()
    };

    // Get authorization URL
    let auth_url = client_info.client.auth_url(&options);

    // Extract state and nonce from the URL
    let state = auth_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| generate_random_string(32));

    let nonce = auth_url
        .query_pairs()
        .find(|(k, _)| k == "nonce")
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| generate_random_string(32));

    // Store session in database
    let expires = Utc::now() + Duration::minutes(10);
    let new_session = NewOidcSession {
        state: state.clone(),
        provider_id: provider_id.to_string(),
        nonce,
        redirect_url: redirect,
        expires: expires.naive_utc(),
    };

    let mut conn = database.get().map_err(|e| {
        tracing::error!("Database connection error: {}", e);
        Status::InternalServerError
    })?;

    diesel::insert_into(oidc_sessions::table)
        .values(&new_session)
        .execute(&mut conn)
        .map_err(|e| {
            tracing::error!("Failed to create OIDC session: {}", e);
            Status::InternalServerError
        })?;

    tracing::debug!("Created OIDC session for provider {}", provider_id);

    Ok(Redirect::to(auth_url.to_string()))
}

/// Handle OIDC callback
#[get("/auth/oidc/<provider_id>/callback?<code>&<state>&<error>&<error_description>")]
pub async fn oidc_callback(
    provider_id: &str,
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
    database: &State<Database>,
    oidc: &State<Arc<OidcClients>>,
    cookies: &CookieJar<'_>,
) -> std::result::Result<Redirect, (Status, String)> {
    // Handle error response from provider
    if let Some(err) = error {
        let desc = error_description.unwrap_or_else(|| "Unknown error".to_string());
        tracing::warn!("OIDC error from provider {}: {} - {}", provider_id, err, desc);
        return Err((Status::BadRequest, format!("OIDC error: {} - {}", err, desc)));
    }

    let code = code.ok_or((Status::BadRequest, "Missing authorization code".to_string()))?;
    let state = state.ok_or((Status::BadRequest, "Missing state parameter".to_string()))?;

    let client_info = oidc.get(provider_id).ok_or_else(|| {
        tracing::warn!("Unknown OIDC provider: {}", provider_id);
        (Status::NotFound, "Unknown provider".to_string())
    })?;

    let mut conn = database.get().map_err(|e| {
        tracing::error!("Database connection error: {}", e);
        (Status::InternalServerError, "Database error".to_string())
    })?;

    // Look up and validate session
    let session: OidcSession = oidc_sessions::table
        .filter(oidc_sessions::state.eq(&state))
        .filter(oidc_sessions::provider_id.eq(provider_id))
        .filter(oidc_sessions::expires.gt(Utc::now().naive_utc()))
        .first(&mut conn)
        .map_err(|_| {
            tracing::warn!("Invalid or expired OIDC session state");
            (Status::BadRequest, "Invalid or expired session".to_string())
        })?;

    // Delete the session (single-use)
    diesel::delete(oidc_sessions::table.filter(oidc_sessions::id.eq(session.id)))
        .execute(&mut conn)
        .ok();

    // Exchange code for token
    let token: Token<StandardClaims> = client_info
        .client
        .authenticate(&code, Some(session.nonce.as_str()), None)
        .await
        .map_err(|e| {
            tracing::error!("Failed to exchange code for tokens: {}", e);
            (Status::InternalServerError, "Token exchange failed".to_string())
        })?;

    // Get user info from token claims
    let userinfo: Option<&Userinfo> = token.id_token
        .as_ref()
        .and_then(|t| t.payload().ok())
        .map(|claims: &StandardClaims| &claims.userinfo);

    let subject = userinfo
        .and_then(|u| u.sub.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let email = userinfo.and_then(|u| u.email.clone());
    let name = extract_name_from_userinfo(userinfo, &client_info.config);

    tracing::debug!(
        "OIDC callback for provider {}: subject={}, email={:?}, name={:?}",
        provider_id,
        subject,
        email,
        name
    );

    // Find or create identity and user
    let (user, auth_token) = find_or_create_user_and_token(
        &mut conn,
        provider_id,
        &subject,
        email.as_deref(),
        name.as_deref(),
        &client_info.config,
    )
    .map_err(|e| {
        tracing::error!("Failed to find or create user: {}", e);
        (Status::InternalServerError, format!("User creation failed: {}", e))
    })?;

    // Set auth cookie with the token
    let mut cookie = Cookie::new("xzar_token", auth_token);
    cookie.set_path("/");
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookies.add(cookie);

    tracing::info!(
        "OIDC login successful for user {} (id: {}) via provider {}",
        user.name,
        user.id,
        provider_id
    );

    // Redirect to the requested URL or default to root
    let redirect_url = session.redirect_url.unwrap_or_else(|| "/".to_string());
    Ok(Redirect::to(redirect_url))
}

/// Extract name from userinfo using configured mapping
fn extract_name_from_userinfo(
    userinfo: Option<&Userinfo>,
    config: &OidcProviderConfig,
) -> Option<String> {
    let userinfo = userinfo?;

    // Try the configured claim
    match config.mapping.name_claim.as_str() {
        "preferred_username" => {
            if let Some(ref username) = userinfo.preferred_username {
                return Some(username.clone());
            }
        }
        "name" => {
            if let Some(ref name) = userinfo.name {
                return Some(name.clone());
            }
        }
        "nickname" => {
            if let Some(ref nickname) = userinfo.nickname {
                return Some(nickname.clone());
            }
        }
        _ => {}
    }

    // Fallback chain: preferred_username -> name -> nickname -> email local part -> subject
    if let Some(ref username) = userinfo.preferred_username {
        return Some(username.clone());
    }

    if let Some(ref name) = userinfo.name {
        return Some(name.clone());
    }

    if let Some(ref nickname) = userinfo.nickname {
        return Some(nickname.clone());
    }

    if let Some(ref email) = userinfo.email {
        if let Some(local) = email.split('@').next() {
            return Some(local.to_string());
        }
    }

    // Last resort: use subject
    userinfo.sub.clone()
}

/// Find or create user and generate token
fn find_or_create_user_and_token(
    conn: &mut PgConnection,
    provider_id: &str,
    subject: &str,
    email: Option<&str>,
    name: Option<&str>,
    config: &OidcProviderConfig,
) -> std::result::Result<(User, String), String> {
    // Look for existing identity
    let existing_identity: Option<OidcIdentity> = oidc_identities::table
        .filter(oidc_identities::provider_id.eq(provider_id))
        .filter(oidc_identities::subject.eq(subject))
        .first(conn)
        .optional()
        .map_err(|e| format!("Database error: {}", e))?;

    let user = if let Some(identity) = existing_identity {
        // Update last login and cached values
        diesel::update(oidc_identities::table.filter(oidc_identities::id.eq(identity.id)))
            .set((
                oidc_identities::last_login.eq(Utc::now().naive_utc()),
                oidc_identities::cached_email.eq(email),
                oidc_identities::cached_name.eq(name),
            ))
            .execute(conn)
            .ok();

        if let Some(user_id) = identity.user_id {
            // Get associated user
            users::table
                .find(user_id)
                .first::<User>(conn)
                .map_err(|e| format!("Failed to find user: {}", e))?
        } else {
            return Err("Identity exists but has no associated user".to_string());
        }
    } else {
        // No existing identity - check if auto-create is enabled
        if !config.auto_create_user {
            return Err("User not found and auto-creation is disabled".to_string());
        }

        // Try to find existing user by email
        let existing_user: Option<User> = if let Some(email) = email {
            users::table
                .filter(users::email.eq(email))
                .first(conn)
                .optional()
                .map_err(|e| format!("Database error: {}", e))?
        } else {
            None
        };

        let user = if let Some(existing) = existing_user {
            // Link to existing user
            existing
        } else {
            // Create new user
            let user_name = generate_unique_username(conn, name, subject)?;

            let new_user = NewUser {
                name: user_name,
                is_admin: config.new_users_admin,
            };

            let user: User = diesel::insert_into(users::table)
                .values(&new_user)
                .get_result(conn)
                .map_err(|e| format!("Failed to create user: {}", e))?;

            // Set email if available
            if let Some(email) = email {
                diesel::update(users::table.filter(users::id.eq(user.id)))
                    .set(users::email.eq(email))
                    .execute(conn)
                    .ok();
            }

            user
        };

        // Create identity record
        let new_identity = NewOidcIdentity {
            provider_id: provider_id.to_string(),
            subject: subject.to_string(),
            user_id: Some(user.id),
            cached_email: email.map(|s| s.to_string()),
            cached_name: name.map(|s| s.to_string()),
        };

        diesel::insert_into(oidc_identities::table)
            .values(&new_identity)
            .execute(conn)
            .map_err(|e| format!("Failed to create identity: {}", e))?;

        user
    };

    // Generate token for the user
    let raw_token = generate_random_token();
    let token_hash = hash_token(&raw_token);

    let new_token = NewToken {
        user_id: Some(user.id),
        token_hash,
        is_system: false,
        description: Some(format!("OIDC login via {}", provider_id)),
    };

    diesel::insert_into(tokens::table)
        .values(&new_token)
        .execute(conn)
        .map_err(|e| format!("Failed to create token: {}", e))?;

    Ok((user, raw_token))
}

/// Generate a unique username
fn generate_unique_username(
    conn: &mut PgConnection,
    preferred_name: Option<&str>,
    subject: &str,
) -> std::result::Result<String, String> {
    let base_name = preferred_name
        .map(|s| sanitize_username(s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("user_{}", &subject[..8.min(subject.len())]));

    // Check if base name is available
    let exists: bool = users::table
        .filter(users::name.eq(&base_name))
        .count()
        .get_result::<i64>(conn)
        .map(|c| c > 0)
        .unwrap_or(false);

    if !exists {
        return Ok(base_name);
    }

    // Try with numeric suffixes
    for i in 1..1000 {
        let candidate = format!("{}_{}", base_name, i);
        let exists: bool = users::table
            .filter(users::name.eq(&candidate))
            .count()
            .get_result::<i64>(conn)
            .map(|c| c > 0)
            .unwrap_or(false);

        if !exists {
            return Ok(candidate);
        }
    }

    Err("Could not generate unique username".to_string())
}

/// Sanitize username to allowed characters
fn sanitize_username(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .take(64)
        .collect()
}

/// Generate random token
fn generate_random_token() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Generate random string for state/nonce
fn generate_random_string(len: usize) -> String {
    use rand::Rng;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Clean up expired OIDC sessions
pub fn cleanup_expired_sessions(conn: &mut PgConnection) -> std::result::Result<usize, diesel::result::Error> {
    diesel::delete(oidc_sessions::table.filter(oidc_sessions::expires.lt(Utc::now().naive_utc())))
        .execute(conn)
}
