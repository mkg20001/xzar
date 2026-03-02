use std::path::Path;

use diesel::prelude::*;
use rocket::post;
use rocket::serde::json::Json;

use crate::auth::AuthenticatedUser;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{CheckRequest, CheckResponse};
use crate::schema::drvs;

/// Extract drv_id (first 32 chars of basename) from a path
fn extract_drv_id(path: &str) -> &str {
    let basename = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    if basename.len() >= 32 {
        &basename[..32]
    } else {
        basename
    }
}

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

    // Extract the drv IDs (first 32 chars of basename)
    let drv_ids: Vec<&str> = paths.iter().map(|p| extract_drv_id(p)).collect();

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
        .filter(|p| !existing_set.contains(extract_drv_id(p)))
        .cloned()
        .collect();

    Ok(Json(CheckResponse { need }))
}
