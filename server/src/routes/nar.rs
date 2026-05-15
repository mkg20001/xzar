use diesel::prelude::*;
use rocket::get;
use rocket::http::ContentType;
use rocket::response::stream::ByteStream;
use rocket::State;

use crate::db::Database;
use crate::error::{AppError, Result};
use crate::models::Drv;
use crate::schema::drvs;
use crate::storage::{Storage, StorageBackend};

/// GET /nar/{filename}
/// Returns the NAR file as a streamed binary response
/// Note: uses Database state instead of Db request guard to avoid holding
/// a DB connection for the entire duration of the streamed response.
#[get("/nar/<filename>")]
pub async fn get_nar(
    filename: &str,
    database: &State<Database>,
    storage: &State<Storage>,
) -> Result<(ContentType, ByteStream![Vec<u8>])> {
    let mut conn = database.get()?;

    // Find the derivation by nar_file
    let drv: Drv = drvs::table
        .filter(drvs::nar_file.eq(filename))
        .first(&mut conn)
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("NAR file not found: {}", filename)))?;

    // Release DB connection before streaming begins
    drop(conn);

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

    // Get the byte stream from storage
    let stream = storage.pull(&drv.nar_file_storage).await?;

    // Convert to Rocket's ByteStream format
    use futures::StreamExt;
    let byte_stream = ByteStream! {
        let mut stream = stream;
        while let Some(result) = stream.next().await {
            match result {
                Ok(bytes) => yield bytes.to_vec(),
                Err(_) => break,
            }
        }
    };

    Ok((content_type, byte_stream))
}
