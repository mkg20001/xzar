use diesel::prelude::*;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use sha2::{Digest, Sha512};

use crate::db::Database;
use crate::models::{AuthenticatedEntity, Token, User};
use crate::schema::{tokens, users};

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
            Some(AuthenticatedEntity::System { token_id: token.id })
        }
        Some((token, Some(user))) if !token.is_system => {
            Some(AuthenticatedEntity::User {
                token_id: token.id,
                user,
            })
        }
        _ => None,
    }
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

        // Try Authorization header first, then fall back to cookie
        let token = request
            .headers()
            .get_one("Authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|s| s.to_string())
            .or_else(|| {
                request
                    .cookies()
                    .get("xzar_token")
                    .map(|c| c.value().to_string())
            });

        match (database, token) {
            (Some(db), Some(raw_token)) => {
                // Try to get a database connection
                match db.get() {
                    Ok(mut conn) => {
                        if let Some(entity) = validate_token(&mut conn, &raw_token) {
                            Outcome::Success(AuthenticatedUser { entity })
                        } else {
                            Outcome::Error((Status::Unauthorized, ()))
                        }
                    }
                    Err(_) => Outcome::Error((Status::ServiceUnavailable, ())),
                }
            }
            (Some(db), None) => {
                // No token provided - check if we should allow dev mode
                match db.get() {
                    Ok(mut conn) => {
                        if !has_any_tokens(&mut conn) {
                            // No tokens in database - development mode
                            tracing::warn!("No tokens in database, allowing unauthenticated access");
                            Outcome::Success(AuthenticatedUser {
                                entity: AuthenticatedEntity::System { token_id: 0 },
                            })
                        } else {
                            Outcome::Error((Status::Unauthorized, ()))
                        }
                    }
                    Err(_) => Outcome::Error((Status::ServiceUnavailable, ())),
                }
            }
            (None, _) => {
                // No database configured - should not happen in production
                tracing::error!("Database not configured");
                Outcome::Error((Status::InternalServerError, ()))
            }
        }
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
