use chrono::Utc;
use diesel::prelude::*;
use rocket::get;
use rocket::http::ContentType;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::Drv;
use crate::schema::drvs;

/// GET /{drvId}.narinfo
/// Returns package metadata in Nix's narinfo format
/// The drv_id is 32 chars, e.g., "abc123...xyz.narinfo"
#[get("/<drv_id_narinfo>", rank = 10)]
pub fn get_narinfo(drv_id_narinfo: String, db: Db) -> Result<(ContentType, String)> {
    // Parse drv_id from "abc123.narinfo" format
    let drv_id = drv_id_narinfo
        .strip_suffix(".narinfo")
        .ok_or_else(|| AppError::NotFound("Invalid narinfo path".to_string()))?;

    let mut conn = db.0;

    // Find the derivation by ID
    let drv: Drv = drvs::table
        .find(drv_id)
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("Derivation not found: {}", drv_id)))?;

    // Update last_fetched timestamp
    diesel::update(drvs::table.find(drv_id))
        .set(drvs::last_fetched.eq(Utc::now().naive_utc()))
        .execute(&mut conn)?;

    // Build the narinfo response
    let mut narinfo = String::new();

    narinfo.push_str(&format!("StorePath: /nix/store/{}\n", drv.drv_full));
    narinfo.push_str(&format!("URL: nar/{}\n", drv.nar_file));

    if let Some(ref comp) = drv.nar_comp {
        narinfo.push_str(&format!("Compression: {}\n", comp));
    }

    narinfo.push_str(&format!("FileHash: {}\n", drv.file_hash));
    narinfo.push_str(&format!("FileSize: {}\n", drv.file_size));
    narinfo.push_str(&format!("NarHash: {}\n", drv.nar_hash));
    narinfo.push_str(&format!("NarSize: {}\n", drv.nar_size));

    // References (filter out None values)
    let refs: Vec<&str> = drv
        .refs
        .iter()
        .filter_map(|r| r.as_ref().map(|s| s.as_str()))
        .collect();
    if !refs.is_empty() {
        narinfo.push_str(&format!("References: {}\n", refs.join(" ")));
    }

    if let Some(ref deriver) = drv.deriver {
        if !deriver.is_empty() {
            narinfo.push_str(&format!("Deriver: {}\n", deriver));
        }
    }

    if let Some(ref sig) = drv.sig {
        narinfo.push_str(&format!("Sig: {}\n", sig));
    }

    Ok((ContentType::Plain, narinfo))
}
