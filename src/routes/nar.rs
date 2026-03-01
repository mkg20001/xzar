use diesel::prelude::*;
use rocket::fs::NamedFile;
use rocket::get;
use rocket::http::ContentType;
use rocket::State;

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::Drv;
use crate::schema::drvs;
use crate::storage::Storage;

/// GET /nar/{filename}
/// Returns the NAR file as a binary stream
#[get("/nar/<filename>")]
pub async fn get_nar(
    filename: &str,
    db: Db,
    storage: &State<Storage>,
) -> Result<(ContentType, NamedFile)> {
    let mut conn = db.0;

    // Find the derivation by nar_file
    let drv: Drv = drvs::table
        .filter(drvs::nar_file.eq(filename))
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("NAR file not found: {}", filename)))?;

    // Check if the file exists in storage
    if !storage.exists(&drv.nar_file_storage).await {
        return Err(AppError::NotFound(format!(
            "NAR file missing from storage: {}",
            filename
        )));
    }

    // Determine content type based on compression
    let content_type = match drv.nar_comp.as_deref() {
        Some("xz") => ContentType::new("application", "x-xz"),
        Some("gzip") | Some("gz") => ContentType::new("application", "gzip"),
        Some("zstd") | Some("zst") => ContentType::new("application", "zstd"),
        Some("bzip2") | Some("bz2") => ContentType::new("application", "x-bzip2"),
        _ => ContentType::Binary,
    };

    // Get the file path and serve it
    let file_path = storage.base_path.join(&drv.nar_file_storage);

    let file = NamedFile::open(&file_path)
        .await
        .map_err(|e| AppError::Io(e))?;

    Ok((content_type, file))
}
