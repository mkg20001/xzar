use diesel::prelude::*;
use rocket::post;
use rocket::serde::json::Json;

use crate::auth::AuthenticatedUser;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{CheckRequest, CheckResponse};
use crate::schema::drvs;

/// POST /check
/// Check which paths are not in the cache
#[post("/check", data = "<request>")]
pub fn check_paths(
    _auth: AuthenticatedUser,
    db: Db,
    request: Json<CheckRequest>,
) -> Result<Json<CheckResponse>> {
    let paths = &request.paths;

    // Limit to 10,000 items
    if paths.len() > 10000 {
        return Err(AppError::BadRequest(
            "Maximum 10,000 paths allowed".to_string(),
        ));
    }

    let mut conn = db.0;

    // Extract the drv IDs (first 32 chars of each path)
    let drv_ids: Vec<&str> = paths
        .iter()
        .map(|p| {
            if p.len() >= 32 {
                &p[..32]
            } else {
                p.as_str()
            }
        })
        .collect();

    // Find existing derivations
    let existing: Vec<String> = drvs::table
        .select(drvs::drv_id)
        .filter(drvs::drv_id.eq_any(&drv_ids))
        .load(&mut conn)?;

    let existing_set: std::collections::HashSet<&str> =
        existing.iter().map(|s| s.as_str()).collect();

    // Return paths that are not in the cache
    let need: Vec<String> = paths
        .iter()
        .filter(|p| {
            let id = if p.len() >= 32 { &p[..32] } else { p.as_str() };
            !existing_set.contains(id)
        })
        .cloned()
        .collect();

    Ok(Json(CheckResponse { need }))
}
