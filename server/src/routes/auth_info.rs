//! Authentication info endpoint

use rocket::get;
use rocket::serde::json::Json;

use crate::auth::AuthenticatedUser;
use crate::models::{SelfResponse, UserInfo};

/// GET /self
/// Returns information about the current authenticated credential
#[get("/self")]
pub fn get_self(auth: AuthenticatedUser) -> Json<SelfResponse> {
    let (credential_type, user) = match &auth.entity {
        crate::models::AuthenticatedEntity::System { .. } => ("system".to_string(), None),
        crate::models::AuthenticatedEntity::User { user, .. } => (
            "user".to_string(),
            Some(UserInfo {
                id: user.id,
                name: user.name.clone(),
            }),
        ),
        crate::models::AuthenticatedEntity::Session { user, .. } => (
            "session".to_string(),
            Some(UserInfo {
                id: user.id,
                name: user.name.clone(),
            }),
        ),
    };

    Json(SelfResponse {
        is_admin: auth.is_admin(),
        credential_type,
        user,
        can_read: auth.entity.can_read(),
        can_write: auth.entity.can_write(),
    })
}
