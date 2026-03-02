use chrono::{Duration, Utc};
use diesel::prelude::*;
use rocket::{delete, get, post};
use rocket::serde::json::Json;

use crate::auth::AuthenticatedUser;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{DrvPin, FinalizePinRequest, NewPin, Pin, PinRoot, PinWithRoots, UpdatePin};
use crate::schema::{drv_pins, drvs, pins};

/// POST /finalizePin
/// Create a named pin (collection) of packages
#[post("/finalizePin", data = "<request>")]
pub fn finalize_pin(
    _auth: AuthenticatedUser,
    db: Db,
    request: Json<FinalizePinRequest>,
) -> Result<Json<i32>> {
    let mut conn = db.0;

    // Validate request
    if request.roots.is_empty() {
        return Err(AppError::BadRequest("At least one root required".to_string()));
    }

    if request.name.is_empty() || request.name.len() > 128 {
        return Err(AppError::BadRequest(
            "Name must be 1-128 characters".to_string(),
        ));
    }

    if let Some(ref desc) = request.desc {
        if desc.len() > 1024 {
            return Err(AppError::BadRequest(
                "Description must be at most 1024 characters".to_string(),
            ));
        }
    }

    // Extract drv IDs and verify all roots exist
    let drv_ids: Vec<&str> = request
        .roots
        .iter()
        .map(|p| {
            if p.len() >= 32 {
                &p[..32]
            } else {
                p.as_str()
            }
        })
        .collect();

    let existing: Vec<String> = drvs::table
        .select(drvs::drv_id)
        .filter(drvs::drv_id.eq_any(&drv_ids))
        .load(&mut conn)?;

    if existing.len() != drv_ids.len() {
        return Err(AppError::BadRequest(
            "Some roots are not in the cache".to_string(),
        ));
    }

    // Mark previous pins with the same name as abandoned
    let now = Utc::now().naive_utc();

    diesel::update(pins::table.filter(pins::name.eq(&request.name)))
        .set((
            pins::abandoned.eq(true),
            pins::expires.eq(Some(now)),
        ))
        .execute(&mut conn)?;

    // Calculate expires timestamp
    let expires = request
        .expires
        .map(|ms| now + Duration::milliseconds(ms));

    // Create new pin
    let new_pin = NewPin {
        name: request.name.clone(),
        description: request.desc.clone(),
        expires,
        leave_after_abandon: request.leave_after_abandon,
    };

    let pin: Pin = diesel::insert_into(pins::table)
        .values(&new_pin)
        .get_result(&mut conn)?;

    // Create drv_pins entries
    let drv_pin_entries: Vec<DrvPin> = drv_ids
        .iter()
        .map(|id| DrvPin {
            drv_id: id.to_string(),
            pin_id: pin.id,
        })
        .collect();

    diesel::insert_into(drv_pins::table)
        .values(&drv_pin_entries)
        .execute(&mut conn)?;

    Ok(Json(pin.id))
}

/// GET /pins
/// List all pins with their roots
#[get("/pins")]
pub fn list_pins(_auth: AuthenticatedUser, db: Db) -> Result<Json<Vec<PinWithRoots>>> {
    let mut conn = db.0;

    // Get all pins ordered by created desc
    let all_pins: Vec<Pin> = pins::table
        .order(pins::created.desc())
        .load(&mut conn)?;

    // For each pin, get its roots
    let mut result = Vec::with_capacity(all_pins.len());

    for pin in all_pins {
        // Join drv_pins with drvs to get full drv info
        let roots: Vec<(String, String)> = drv_pins::table
            .inner_join(drvs::table)
            .filter(drv_pins::pin_id.eq(pin.id))
            .select((drvs::drv_id, drvs::drv_full))
            .load(&mut conn)?;

        let pin_roots: Vec<PinRoot> = roots
            .into_iter()
            .map(|(drv_id, drv_full)| PinRoot { drv_id, drv_full })
            .collect();

        result.push(PinWithRoots {
            id: pin.id,
            name: pin.name,
            description: pin.description,
            created: pin.created,
            expires: pin.expires,
            abandoned: pin.abandoned,
            leave_after_abandon: pin.leave_after_abandon,
            roots: pin_roots,
        });
    }

    Ok(Json(result))
}

/// DELETE /pins/<id>
/// Abandon a pin (mark as abandoned and set expires to now)
#[delete("/pins/<id>")]
pub fn abandon_pin(_auth: AuthenticatedUser, db: Db, id: i32) -> Result<Json<bool>> {
    let mut conn = db.0;

    let now = Utc::now().naive_utc();

    let updated = diesel::update(pins::table.filter(pins::id.eq(id)))
        .set(UpdatePin {
            abandoned: Some(true),
            expires: Some(Some(now)),
        })
        .execute(&mut conn)?;

    if updated == 0 {
        return Err(AppError::NotFound("Pin not found".to_string()));
    }

    Ok(Json(true))
}
