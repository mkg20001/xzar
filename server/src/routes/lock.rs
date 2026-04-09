use chrono::{Duration, Utc};
use diesel::prelude::*;
use rocket::post;
use rocket::serde::json::Json;

use crate::auth::WriteUser;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{Lock, LockClearRequest, LockExtendRequest, LockResponse, NewLock, OkResponse};
use crate::schema::{drv_locks, locks};

const LOCK_TTL_DAYS: i64 = 2;

/// POST /lock/request
/// Request a new upload lock (2-day TTL)
#[post("/lock/request")]
pub fn lock_request(_auth: WriteUser, db: Db) -> Result<Json<LockResponse>> {
    let mut conn = db.0;

    let new_lock = NewLock { owner: 1 };

    let lock: Lock = diesel::insert_into(locks::table)
        .values(&new_lock)
        .get_result(&mut conn)?;

    Ok(Json(LockResponse {
        lock: lock.id,
        deadline: lock.expires,
    }))
}

/// POST /lock/extend
/// Extend a lock's deadline by 2 more days
#[post("/lock/extend", data = "<request>")]
pub fn lock_extend(
    _auth: WriteUser,
    db: Db,
    request: Json<LockExtendRequest>,
) -> Result<Json<LockResponse>> {
    let mut conn = db.0;

    let new_expires = Utc::now().naive_utc() + Duration::days(LOCK_TTL_DAYS);

    let lock: Lock = diesel::update(locks::table.find(request.lock))
        .set(locks::expires.eq(new_expires))
        .get_result(&mut conn)
        .optional()?
        .ok_or(AppError::LockNotFound)?;

    Ok(Json(LockResponse {
        lock: lock.id,
        deadline: lock.expires,
    }))
}

/// POST /lock/clear
/// Release/clear a lock
#[post("/lock/clear", data = "<request>")]
pub fn lock_clear(
    _auth: WriteUser,
    db: Db,
    request: Json<LockClearRequest>,
) -> Result<Json<OkResponse>> {
    let mut conn = db.0;

    // Delete associated drv_locks entries (cascade should handle this, but be explicit)
    diesel::delete(drv_locks::table.filter(drv_locks::lock_id.eq(request.lock)))
        .execute(&mut conn)?;

    // Delete the lock
    let deleted = diesel::delete(locks::table.find(request.lock)).execute(&mut conn)?;

    if deleted == 0 {
        return Err(AppError::LockNotFound);
    }

    Ok(Json(OkResponse { ok: true }))
}
