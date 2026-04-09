//! Admin management endpoints
//!
//! All routes require AdminUser (admin privileges)

use diesel::prelude::*;
use rocket::serde::json::Json;
use rocket::{delete, get, patch, post, put};

use crate::auth::{hash_token, AdminUser};
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{
    AdminTokenResponse, AdminUserResponse, CreateTokenRequest, CreateTokenResponse,
    CreateUserRequest, NewToken, NewUser, Token, UpdateTokenRequest, UpdateUserRequest, User,
};
use crate::schema::{tokens, users};

// ============ Users ============

/// GET /admin/users
/// List all users
#[get("/admin/users")]
pub fn list_users(_admin: AdminUser, db: Db) -> Result<Json<Vec<AdminUserResponse>>> {
    let mut conn = db.0;

    let all_users: Vec<User> = users::table.order(users::id.asc()).load(&mut conn)?;

    let response: Vec<AdminUserResponse> = all_users
        .into_iter()
        .map(|u| AdminUserResponse {
            id: u.id,
            name: u.name,
            email: u.email,
            is_admin: u.is_admin,
            created: u.created.to_string(),
        })
        .collect();

    Ok(Json(response))
}

/// POST /admin/users
/// Create a new user
#[post("/admin/users", data = "<request>")]
pub fn create_user(
    _admin: AdminUser,
    db: Db,
    request: Json<CreateUserRequest>,
) -> Result<Json<AdminUserResponse>> {
    let mut conn = db.0;

    // Validate name
    if request.name.is_empty() || request.name.len() > 128 {
        return Err(AppError::BadRequest(
            "Name must be 1-128 characters".to_string(),
        ));
    }

    let new_user = NewUser {
        name: request.name.clone(),
        is_admin: request.is_admin,
    };

    let user: User = diesel::insert_into(users::table)
        .values(&new_user)
        .get_result(&mut conn)?;

    Ok(Json(AdminUserResponse {
        id: user.id,
        name: user.name,
        email: user.email,
        is_admin: user.is_admin,
        created: user.created.to_string(),
    }))
}

/// PATCH /admin/users/<id>
/// Update a user (PATCH style - only specified fields are updated)
#[patch("/admin/users/<id>", data = "<request>")]
pub fn update_user(
    _admin: AdminUser,
    db: Db,
    id: i32,
    request: Json<UpdateUserRequest>,
) -> Result<Json<AdminUserResponse>> {
    let mut conn = db.0;

    // Check if user exists
    let user_exists = users::table
        .filter(users::id.eq(id))
        .count()
        .get_result::<i64>(&mut conn)
        .map(|count| count > 0)?;

    if !user_exists {
        return Err(AppError::NotFound("User not found".to_string()));
    }

    // Validate name if provided
    if let Some(ref name) = request.name {
        if name.is_empty() || name.len() > 128 {
            return Err(AppError::BadRequest(
                "Name must be 1-128 characters".to_string(),
            ));
        }
    }

    // Validate email if provided
    if let Some(Some(ref email)) = request.email {
        if email.len() > 256 {
            return Err(AppError::BadRequest(
                "Email must be at most 256 characters".to_string(),
            ));
        }
    }

    // Update name if specified
    if let Some(ref name) = request.name {
        diesel::update(users::table.filter(users::id.eq(id)))
            .set(users::name.eq(name))
            .execute(&mut conn)?;
    }

    // Update is_admin if specified
    if let Some(is_admin) = request.is_admin {
        diesel::update(users::table.filter(users::id.eq(id)))
            .set(users::is_admin.eq(is_admin))
            .execute(&mut conn)?;
    }

    // Update email if specified
    if let Some(ref email) = request.email {
        diesel::update(users::table.filter(users::id.eq(id)))
            .set(users::email.eq(email))
            .execute(&mut conn)?;
    }

    let user: User = users::table.find(id).first(&mut conn)?;

    Ok(Json(AdminUserResponse {
        id: user.id,
        name: user.name,
        email: user.email,
        is_admin: user.is_admin,
        created: user.created.to_string(),
    }))
}

/// DELETE /admin/users/<id>
/// Delete a user
#[delete("/admin/users/<id>")]
pub fn delete_user(_admin: AdminUser, db: Db, id: i32) -> Result<Json<bool>> {
    let mut conn = db.0;

    let deleted = diesel::delete(users::table.filter(users::id.eq(id))).execute(&mut conn)?;

    if deleted == 0 {
        return Err(AppError::NotFound("User not found".to_string()));
    }

    Ok(Json(true))
}

// ============ Tokens ============

/// GET /admin/tokens
/// List all tokens (with user info)
#[get("/admin/tokens")]
pub fn list_tokens(_admin: AdminUser, db: Db) -> Result<Json<Vec<AdminTokenResponse>>> {
    let mut conn = db.0;

    let all_tokens: Vec<(Token, Option<User>)> = tokens::table
        .left_join(users::table)
        .order(tokens::id.asc())
        .select((Token::as_select(), Option::<User>::as_select()))
        .load(&mut conn)?;

    let response: Vec<AdminTokenResponse> = all_tokens
        .into_iter()
        .map(|(token, user)| AdminTokenResponse {
            id: token.id,
            user_id: token.user_id,
            user_name: user.map(|u| u.name),
            is_system: token.is_system,
            description: token.description,
            created: token.created.to_string(),
            can_read: token.can_read,
            can_write: token.can_write,
        })
        .collect();

    Ok(Json(response))
}

/// POST /admin/tokens
/// Create a new token
#[post("/admin/tokens", data = "<request>")]
pub fn create_token(
    _admin: AdminUser,
    db: Db,
    request: Json<CreateTokenRequest>,
) -> Result<Json<CreateTokenResponse>> {
    let mut conn = db.0;

    // Determine if this is a system token or user token
    let is_system = request.user_id.is_none();

    // Resolve permissions (default both true if not specified)
    let can_read = request.can_read.unwrap_or(true);
    let can_write = request.can_write.unwrap_or(true);

    if !can_read && !can_write {
        return Err(AppError::BadRequest(
            "Token must have at least one of can_read or can_write".to_string(),
        ));
    }

    // If user_id is provided, verify user exists
    if let Some(user_id) = request.user_id {
        let user_exists: bool = users::table
            .filter(users::id.eq(user_id))
            .count()
            .get_result::<i64>(&mut conn)
            .map(|count| count > 0)?;

        if !user_exists {
            return Err(AppError::BadRequest("User not found".to_string()));
        }
    }

    // Generate random token
    let raw_token: String = {
        use rand::Rng;
        let bytes: [u8; 32] = rand::thread_rng().gen();
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    };

    let token_hash = hash_token(&raw_token);

    let new_token = NewToken {
        user_id: request.user_id,
        token_hash,
        is_system,
        description: request.description.clone(),
        can_read,
        can_write,
    };

    let token: Token = diesel::insert_into(tokens::table)
        .values(&new_token)
        .get_result(&mut conn)?;

    Ok(Json(CreateTokenResponse {
        id: token.id,
        token: raw_token,
    }))
}

/// PUT /admin/tokens/<id>
/// Update a token (description only)
#[put("/admin/tokens/<id>", data = "<request>")]
pub fn update_token(
    _admin: AdminUser,
    db: Db,
    id: i32,
    request: Json<UpdateTokenRequest>,
) -> Result<Json<AdminTokenResponse>> {
    let mut conn = db.0;

    // Check token exists
    let existing: Token = tokens::table
        .find(id)
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| AppError::NotFound("Token not found".to_string()))?;

    // Update description if provided
    if request.description.is_some() {
        diesel::update(tokens::table.filter(tokens::id.eq(id)))
            .set(tokens::description.eq(&request.description))
            .execute(&mut conn)?;
    }

    // Update permissions if provided
    if request.can_read.is_some() || request.can_write.is_some() {
        let new_read = request.can_read.unwrap_or(existing.can_read);
        let new_write = request.can_write.unwrap_or(existing.can_write);

        if !new_read && !new_write {
            return Err(AppError::BadRequest(
                "Token must have at least one of can_read or can_write".to_string(),
            ));
        }

        diesel::update(tokens::table.filter(tokens::id.eq(id)))
            .set((tokens::can_read.eq(new_read), tokens::can_write.eq(new_write)))
            .execute(&mut conn)?;
    }

    // Fetch updated token with user info
    let (token, user): (Token, Option<User>) = tokens::table
        .left_join(users::table)
        .filter(tokens::id.eq(id))
        .select((Token::as_select(), Option::<User>::as_select()))
        .first(&mut conn)?;

    Ok(Json(AdminTokenResponse {
        id: token.id,
        user_id: token.user_id,
        user_name: user.map(|u| u.name),
        is_system: token.is_system,
        description: token.description,
        created: token.created.to_string(),
        can_read: token.can_read,
        can_write: token.can_write,
    }))
}

/// DELETE /admin/tokens/<id>
/// Delete/revoke a token
#[delete("/admin/tokens/<id>")]
pub fn delete_token(_admin: AdminUser, db: Db, id: i32) -> Result<Json<bool>> {
    let mut conn = db.0;

    let deleted = diesel::delete(tokens::table.filter(tokens::id.eq(id))).execute(&mut conn)?;

    if deleted == 0 {
        return Err(AppError::NotFound("Token not found".to_string()));
    }

    Ok(Json(true))
}
