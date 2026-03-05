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
use openid::biscuit::jwk::JWKSet;
use openid::biscuit::Empty;
use openid::{Client, Config as OidcConfig, Discovered, Options, StandardClaims, Token, Userinfo};
use rocket::http::{Cookie, CookieJar, SameSite, Status};
use rocket::response::Redirect;
use rocket::serde::json::Json;
use rocket::{get, State};
use serde::Serialize;
use url::Url;

use crate::auth::hash_token;
use crate::config::{Config, OidcProviderConfig};
use crate::db::Database;
use crate::models::{NewOidcIdentity, NewOidcSession, NewSession, NewUser, OidcIdentity, OidcSession, User};
use crate::schema::{oidc_identities, oidc_sessions, sessions, users};

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

            let client = if provider_config.discover {
                // Use OIDC discovery
                Client::<Discovered, StandardClaims>::discover(
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
                })?
            } else {
                // Manual endpoint configuration
                let authorization_endpoint = provider_config
                    .authorization_endpoint
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "authorization_endpoint required when discover=false for {}",
                            provider_config.id
                        )
                    })?;
                let token_endpoint = provider_config.token_endpoint.as_ref().ok_or_else(|| {
                    format!(
                        "token_endpoint required when discover=false for {}",
                        provider_config.id
                    )
                })?;

                let auth_endpoint = Url::parse(authorization_endpoint).map_err(|e| {
                    format!(
                        "Invalid authorization_endpoint for {}: {}",
                        provider_config.id, e
                    )
                })?;
                let token_ep = Url::parse(token_endpoint).map_err(|e| {
                    format!("Invalid token_endpoint for {}: {}", provider_config.id, e)
                })?;
                let jwks_url = provider_config
                    .jwks_uri
                    .as_ref()
                    .map(|u| Url::parse(u))
                    .transpose()
                    .map_err(|e| {
                        format!("Invalid jwks_uri for {}: {}", provider_config.id, e)
                    })?;
                let userinfo_ep = provider_config
                    .userinfo_endpoint
                    .as_ref()
                    .map(|u| Url::parse(u))
                    .transpose()
                    .map_err(|e| {
                        format!("Invalid userinfo_endpoint for {}: {}", provider_config.id, e)
                    })?;

                // Use a dummy JWKS URI if not provided (won't be used since we won't have JWKS)
                let config_jwks_uri = jwks_url
                    .clone()
                    .unwrap_or_else(|| issuer.clone());

                // Create OIDC config manually
                let oidc_config = OidcConfig {
                    issuer: issuer.clone(),
                    authorization_endpoint: auth_endpoint,
                    token_endpoint: token_ep,
                    userinfo_endpoint: userinfo_ep,
                    jwks_uri: config_jwks_uri,
                    // Required fields with sensible defaults
                    response_types_supported: vec!["code".to_string()],
                    subject_types_supported: vec!["public".to_string()],
                    id_token_signing_alg_values_supported: vec!["RS256".to_string()],
                    // Optional fields
                    introspection_endpoint: None,
                    end_session_endpoint: None,
                    registration_endpoint: None,
                    scopes_supported: Some(provider_config.scopes.clone()),
                    response_modes_supported: None,
                    grant_types_supported: Some(vec!["authorization_code".to_string()]),
                    acr_values_supported: None,
                    id_token_encryption_alg_values_supported: None,
                    id_token_encryption_enc_values_supported: None,
                    userinfo_signing_alg_values_supported: None,
                    userinfo_encryption_alg_values_supported: None,
                    userinfo_encryption_enc_values_supported: None,
                    request_object_signing_alg_values_supported: None,
                    request_object_encryption_alg_values_supported: None,
                    request_object_encryption_enc_values_supported: None,
                    token_endpoint_auth_methods_supported: None,
                    token_endpoint_auth_signing_alg_values_supported: None,
                    display_values_supported: None,
                    claim_types_supported: None,
                    claims_supported: None,
                    service_documentation: None,
                    claims_locales_supported: None,
                    ui_locales_supported: None,
                    claims_parameter_supported: false,
                    request_parameter_supported: false,
                    request_uri_parameter_supported: true,
                    require_request_uri_registration: false,
                    op_policy_uri: None,
                    op_tos_uri: None,
                    code_challenge_methods_supported: None,
                };

                let http_client = reqwest::Client::new();

                // Fetch JWKS only if jwks_uri is provided
                let jwks: Option<JWKSet<Empty>> = if let Some(url) = jwks_url {
                    let jwks_data: JWKSet<Empty> = http_client
                        .get(url)
                        .send()
                        .await
                        .map_err(|e| format!("Failed to fetch JWKS for {}: {}", provider_config.id, e))?
                        .json()
                        .await
                        .map_err(|e| {
                            format!("Failed to parse JWKS for {}: {}", provider_config.id, e)
                        })?;
                    Some(jwks_data)
                } else {
                    tracing::info!(
                        "No jwks_uri configured for provider {}, token signatures will not be verified",
                        provider_config.id
                    );
                    None
                };

                // Create provider from config
                let provider: Discovered = oidc_config.into();

                Client::new(
                    provider,
                    provider_config.client_id.clone(),
                    Some(provider_config.client_secret.clone()),
                    Some(redirect_url),
                    http_client,
                    jwks,
                )
            };

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

    // Generate state and nonce for CSRF protection and token validation
    let state = generate_random_string(32);
    let nonce = generate_random_string(32);

    // Build options with scopes, state, and nonce
    let options = Options {
        scope: Some(client_info.config.scopes.join(" ")),
        state: Some(state.clone()),
        nonce: Some(nonce.clone()),
        ..Default::default()
    };

    // Get authorization URL (includes state and nonce)
    let auth_url = client_info.client.auth_url(&options);

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

    // Get userinfo - either from ID token claims or by fetching from userinfo endpoint
    let userinfo: Option<Userinfo> = if let Some(ref id_token) = token.id_token {
        // OIDC flow - get claims from ID token
        let claims = id_token.payload().ok();
        tracing::debug!(
            "OIDC provider {} id_token claims: {:?}",
            provider_id,
            claims
        );
        claims.map(|c: &StandardClaims| c.userinfo.clone())
    } else {
        // OAuth2 flow (e.g., GitHub) - fetch from userinfo endpoint
        tracing::debug!(
            "OIDC provider {} has no id_token, fetching from userinfo endpoint",
            provider_id
        );

        // First try the openid crate's native method
        match client_info.client.request_userinfo(&token).await {
            Ok(info) => {
                tracing::debug!(
                    "OIDC provider {} userinfo response: {:?}",
                    provider_id,
                    info
                );
                Some(info)
            }
            Err(e) => {
                // Fallback: fetch manually with custom headers (some providers like GitHub require this)
                tracing::debug!(
                    "OIDC provider {} native userinfo fetch failed ({}), trying fallback",
                    provider_id,
                    e
                );
                fetch_userinfo_fallback(&client_info.client, &token, provider_id).await
            }
        }
    };

    let userinfo_ref = userinfo.as_ref();

    // Extract subject (use "unknown" as fallback)
    let subject = userinfo_ref
        .and_then(|u| u.sub.clone())
        .unwrap_or_else(|| {
            tracing::warn!("OIDC provider {} did not return a subject claim, using 'unknown'", provider_id);
            "unknown".to_string()
        });

    // Extract email using configured mapping
    let email = extract_email_from_userinfo(userinfo_ref, &client_info.config, provider_id);

    // Extract name using configured mapping
    let name = extract_name_from_userinfo(userinfo_ref, &client_info.config, provider_id);

    // Fail if we cannot establish identity (no name AND no email)
    if name.is_none() && email.is_none() {
        tracing::error!(
            "OIDC provider {} did not return enough user information. \
             Neither name (claim: {}) nor email (claim: {}) could be extracted. \
             Subject: {}",
            provider_id,
            client_info.config.mapping.name_claim,
            client_info.config.mapping.email_claim,
            subject
        );
        return Err((
            Status::BadRequest,
            format!(
                "Cannot establish user identity: no name or email available from provider. \
                 Check that the provider returns the configured claims ({}, {}).",
                client_info.config.mapping.name_claim,
                client_info.config.mapping.email_claim
            ),
        ));
    }

    tracing::debug!(
        "OIDC callback for provider {}: subject={}, email={:?}, name={:?}",
        provider_id,
        subject,
        email,
        name
    );

    // Find or create identity and user, then create session
    let (user, session_token) = find_or_create_user_and_session(
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

    // Set session cookie
    let mut cookie = Cookie::new("xzar_session", session_token);
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

/// Extract email from userinfo using configured mapping
fn extract_email_from_userinfo(
    userinfo: Option<&Userinfo>,
    config: &OidcProviderConfig,
    provider_id: &str,
) -> Option<String> {
    let userinfo = userinfo?;
    let claim = &config.mapping.email_claim;

    let result = match claim.as_str() {
        "email" => userinfo.email.clone(),
        other => {
            tracing::error!(
                "OIDC provider {}: unknown email claim '{}', only 'email' is supported",
                provider_id,
                other
            );
            None
        }
    };

    if result.is_none() {
        tracing::warn!(
            "OIDC provider {}: email claim '{}' not present in token",
            provider_id,
            claim
        );
    }

    result
}

/// Extract name from userinfo using configured mapping
fn extract_name_from_userinfo(
    userinfo: Option<&Userinfo>,
    config: &OidcProviderConfig,
    provider_id: &str,
) -> Option<String> {
    let userinfo = userinfo?;
    let claim = &config.mapping.name_claim;

    // Try the configured claim first
    let result = match claim.as_str() {
        "preferred_username" => userinfo.preferred_username.clone(),
        "name" => userinfo.name.clone(),
        "nickname" => userinfo.nickname.clone(),
        "email" => userinfo.email.as_ref().and_then(|e| {
            e.split('@').next().map(|s| s.to_string())
        }),
        "sub" => userinfo.sub.clone(),
        other => {
            tracing::error!(
                "OIDC provider {}: unknown name claim '{}', supported claims are: \
                 preferred_username, name, nickname, email, sub",
                provider_id,
                other
            );
            None
        }
    };

    if result.is_some() {
        return result;
    }

    // Log that configured claim was not found
    tracing::warn!(
        "OIDC provider {}: name claim '{}' not present in token, trying fallbacks",
        provider_id,
        claim
    );

    // Fallback chain: preferred_username -> name -> nickname -> email local part -> subject
    if let Some(ref username) = userinfo.preferred_username {
        tracing::debug!("OIDC provider {}: using fallback claim 'preferred_username'", provider_id);
        return Some(username.clone());
    }

    if let Some(ref name) = userinfo.name {
        tracing::debug!("OIDC provider {}: using fallback claim 'name'", provider_id);
        return Some(name.clone());
    }

    if let Some(ref nickname) = userinfo.nickname {
        tracing::debug!("OIDC provider {}: using fallback claim 'nickname'", provider_id);
        return Some(nickname.clone());
    }

    if let Some(ref email) = userinfo.email {
        if let Some(local) = email.split('@').next() {
            tracing::debug!("OIDC provider {}: using email local part as name fallback", provider_id);
            return Some(local.to_string());
        }
    }

    if let Some(ref sub) = userinfo.sub {
        tracing::debug!("OIDC provider {}: using subject as name fallback", provider_id);
        return Some(sub.clone());
    }

    tracing::error!(
        "OIDC provider {}: no name claim could be extracted from token",
        provider_id
    );
    None
}

/// Fallback userinfo fetch with custom headers for providers like GitHub
async fn fetch_userinfo_fallback(
    client: &Client<Discovered, StandardClaims>,
    token: &Token<StandardClaims>,
    provider_id: &str,
) -> Option<Userinfo> {
    let userinfo_url = client.config().userinfo_endpoint.as_ref()?;
    let access_token = &token.bearer.access_token;
    let http_client = reqwest::Client::new();

    let response = match http_client
        .get(userinfo_url.clone())
        .header("Authorization", format!("Bearer {}", access_token))
        .header("User-Agent", "xzar-server")
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(
                "OIDC provider {} fallback userinfo fetch failed: {}",
                provider_id,
                e
            );
            return None;
        }
    };

    if !response.status().is_success() {
        tracing::error!(
            "OIDC provider {} fallback userinfo request failed: {}",
            provider_id,
            response.status()
        );
        return None;
    }

    let text = match response.text().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                "OIDC provider {} failed to read userinfo response: {}",
                provider_id,
                e
            );
            return None;
        }
    };

    tracing::debug!(
        "OIDC provider {} raw userinfo response: {}",
        provider_id,
        text
    );

    // Try to parse as standard Userinfo first
    if let Ok(info) = serde_json::from_str::<Userinfo>(&text) {
        return Some(info);
    }

    // Try parsing as generic JSON and map common OAuth2 fields
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(json) => {
            let sub = json.get("id")
                .and_then(|v| v.as_i64())
                .map(|id| id.to_string())
                .or_else(|| json.get("sub").and_then(|v| v.as_str()).map(|s| s.to_string()));
            let preferred_username = json.get("login")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let name = json.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let email = json.get("email")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let info = Userinfo {
                sub,
                name,
                email,
                preferred_username,
                given_name: None,
                family_name: None,
                middle_name: None,
                nickname: None,
                profile: None,
                picture: None,
                website: None,
                email_verified: false,
                gender: None,
                birthdate: None,
                zoneinfo: None,
                locale: None,
                phone_number: None,
                phone_number_verified: false,
                address: None,
                updated_at: None,
            };
            tracing::debug!(
                "OIDC provider {} mapped userinfo: {:?}",
                provider_id,
                info
            );
            Some(info)
        }
        Err(e) => {
            tracing::error!(
                "OIDC provider {} failed to parse userinfo as JSON: {}",
                provider_id,
                e
            );
            None
        }
    }
}

/// Find or create user and generate session token
fn find_or_create_user_and_session(
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

    // Generate session token (expires in 7 days)
    let raw_token = generate_random_token();
    let token_hash = hash_token(&raw_token);
    let expires = Utc::now() + Duration::days(7);

    let new_session = NewSession {
        user_id: user.id,
        token_hash,
        expires: expires.naive_utc(),
    };

    diesel::insert_into(sessions::table)
        .values(&new_session)
        .execute(conn)
        .map_err(|e| format!("Failed to create session: {}", e))?;

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
pub fn cleanup_expired_oidc_sessions(conn: &mut PgConnection) -> std::result::Result<usize, diesel::result::Error> {
    diesel::delete(oidc_sessions::table.filter(oidc_sessions::expires.lt(Utc::now().naive_utc())))
        .execute(conn)
}

/// Clean up expired user sessions
pub fn cleanup_expired_sessions(conn: &mut PgConnection) -> std::result::Result<usize, diesel::result::Error> {
    diesel::delete(sessions::table.filter(sessions::expires.lt(Utc::now().naive_utc())))
        .execute(conn)
}
