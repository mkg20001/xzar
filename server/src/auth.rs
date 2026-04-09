use chrono::Utc;
use diesel::prelude::*;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use sha2::{Digest, Sha512};

use crate::db::Database;
use crate::models::{AuthenticatedEntity, Session, Token, User};
use crate::schema::{sessions, tokens, users};

/// Request guard for authenticated requests
/// Contains information about the authenticated entity
pub struct AuthenticatedUser {
    pub entity: AuthenticatedEntity,
}

impl AuthenticatedUser {
    pub fn is_admin(&self) -> bool {
        self.entity.is_admin()
    }

    pub fn user(&self) -> Option<&User> {
        self.entity.user()
    }
}

/// Request guard that requires admin privileges
pub struct AdminUser {
    pub entity: AuthenticatedEntity,
}

/// Hash a raw token using SHA-512
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha512::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(HEX_CHARS[(byte >> 4) as usize] as char);
        hex.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    hex
}

/// Validate token against database
/// Returns Some(AuthenticatedEntity) if valid, None otherwise
fn validate_token(
    conn: &mut diesel::PgConnection,
    raw_token: &str,
) -> Option<AuthenticatedEntity> {
    let token_hash = hash_token(raw_token);

    // Query for matching token with optional user join
    let result: Option<(Token, Option<User>)> = tokens::table
        .left_join(users::table)
        .filter(tokens::token_hash.eq(&token_hash))
        .select((Token::as_select(), Option::<User>::as_select()))
        .first(conn)
        .optional()
        .ok()?;

    match result {
        Some((token, None)) if token.is_system => {
            Some(AuthenticatedEntity::System {
                token_id: token.id,
                can_read: token.can_read,
                can_write: token.can_write,
            })
        }
        Some((token, Some(user))) if !token.is_system => {
            Some(AuthenticatedEntity::User {
                token_id: token.id,
                user,
                can_read: token.can_read,
                can_write: token.can_write,
            })
        }
        _ => None,
    }
}

/// Validate session token against database (for cookie-based auth)
/// Returns Some(AuthenticatedEntity) if valid and not expired, None otherwise
fn validate_session(
    conn: &mut diesel::PgConnection,
    raw_token: &str,
) -> Option<AuthenticatedEntity> {
    let token_hash = hash_token(raw_token);

    // Query for matching session with user join, checking expiry
    let result: Option<(Session, User)> = sessions::table
        .inner_join(users::table)
        .filter(sessions::token_hash.eq(&token_hash))
        .filter(sessions::expires.gt(Utc::now().naive_utc()))
        .select((Session::as_select(), User::as_select()))
        .first(conn)
        .optional()
        .ok()?;

    result.map(|(session, user)| AuthenticatedEntity::Session {
        session_id: session.id,
        user,
    })
}

/// Check if there are any tokens in the database
fn has_any_tokens(conn: &mut diesel::PgConnection) -> bool {
    tokens::table
        .count()
        .get_result::<i64>(conn)
        .map(|count| count > 0)
        .unwrap_or(false)
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AuthenticatedUser {
    type Error = ();

    async fn from_request(request: &'r rocket::Request<'_>) -> Outcome<Self, Self::Error> {
        let database = request.rocket().state::<Database>();

        // Try Authorization header first (xzar token)
        let auth_header_token = request
            .headers()
            .get_one("Authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|s| s.to_string());

        // Try xzar_token cookie (xzar token)
        let xzar_cookie_token = request
            .cookies()
            .get("xzar_token")
            .map(|c| c.value().to_string());

        // Try xzar_session cookie (session token from OIDC)
        let session_cookie_token = request
            .cookies()
            .get("xzar_session")
            .map(|c| c.value().to_string());

        let db = match database {
            Some(db) => db,
            None => {
                tracing::error!("Database not configured");
                return Outcome::Error((Status::InternalServerError, ()));
            }
        };

        let mut conn = match db.get() {
            Ok(conn) => conn,
            Err(_) => return Outcome::Error((Status::ServiceUnavailable, ())),
        };

        // Priority: Authorization header > xzar_token cookie > xzar_session cookie
        if let Some(raw_token) = auth_header_token.or(xzar_cookie_token) {
            tracing::debug!("Auth: validating xzar token");
            if let Some(entity) = validate_token(&mut conn, &raw_token) {
                tracing::debug!("Auth: xzar token valid");
                return Outcome::Success(AuthenticatedUser { entity });
            }
            tracing::debug!("Auth: xzar token invalid");
            return Outcome::Error((Status::Unauthorized, ()));
        }

        // Try session token from cookie
        if let Some(raw_token) = session_cookie_token {
            tracing::debug!("Auth: validating session cookie");
            if let Some(entity) = validate_session(&mut conn, &raw_token) {
                tracing::debug!("Auth: session valid for user {:?}", entity.user().map(|u| &u.name));
                return Outcome::Success(AuthenticatedUser { entity });
            }
            tracing::debug!("Auth: session invalid or expired");
            return Outcome::Error((Status::Unauthorized, ()));
        }

        tracing::debug!("Auth: no token or session provided");

        // No token provided - check if we should allow dev mode
        if !has_any_tokens(&mut conn) {
            // No tokens in database - development mode
            tracing::warn!("No tokens in database, allowing unauthenticated access");
            return Outcome::Success(AuthenticatedUser {
                entity: AuthenticatedEntity::System { token_id: 0, can_read: true, can_write: true },
            });
        }

        Outcome::Error((Status::Unauthorized, ()))
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AdminUser {
    type Error = ();

    async fn from_request(request: &'r rocket::Request<'_>) -> Outcome<Self, Self::Error> {
        match AuthenticatedUser::from_request(request).await {
            Outcome::Success(auth) if auth.is_admin() => {
                Outcome::Success(AdminUser { entity: auth.entity })
            }
            Outcome::Success(_) => Outcome::Error((Status::Forbidden, ())),
            Outcome::Error(e) => Outcome::Error(e),
            Outcome::Forward(f) => Outcome::Forward(f),
        }
    }
}

/// Request guard that requires read permission
pub struct ReadUser {
    pub entity: AuthenticatedEntity,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for ReadUser {
    type Error = ();

    async fn from_request(request: &'r rocket::Request<'_>) -> Outcome<Self, Self::Error> {
        match AuthenticatedUser::from_request(request).await {
            Outcome::Success(auth) if auth.entity.can_read() => {
                Outcome::Success(ReadUser { entity: auth.entity })
            }
            Outcome::Success(_) => Outcome::Error((Status::Forbidden, ())),
            Outcome::Error(e) => Outcome::Error(e),
            Outcome::Forward(f) => Outcome::Forward(f),
        }
    }
}

/// Request guard that requires write permission
pub struct WriteUser {
    pub entity: AuthenticatedEntity,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for WriteUser {
    type Error = ();

    async fn from_request(request: &'r rocket::Request<'_>) -> Outcome<Self, Self::Error> {
        match AuthenticatedUser::from_request(request).await {
            Outcome::Success(auth) if auth.entity.can_write() => {
                Outcome::Success(WriteUser { entity: auth.entity })
            }
            Outcome::Success(_) => Outcome::Error((Status::Forbidden, ())),
            Outcome::Error(e) => Outcome::Error(e),
            Outcome::Forward(f) => Outcome::Forward(f),
        }
    }
}
