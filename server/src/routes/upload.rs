use base64::Engine;
use diesel::prelude::*;
use futures::TryStreamExt;
use rocket::data::{Data, ToByteUnit};
use rocket::http::ContentType;
use rocket::put;
use rocket::serde::json::Json;
use rocket::State;
use tokio_util::io::StreamReader;

use crate::auth::AuthenticatedUser;
use crate::config::Config;
use crate::crypto::{parse_hash, NixSigningKey};
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{DrvLock, NewDrv, OkResponse};
use crate::schema::{drv_locks, drvs, locks};
use crate::storage::{Storage, StorageBackend};

/// PUT /uploadNar
/// Upload a NAR file with metadata (multipart form)
/// Streams file data directly to storage to avoid memory buildup.
#[put("/uploadNar", data = "<data>")]
pub async fn upload_nar(
    _auth: AuthenticatedUser,
    content_type: &ContentType,
    db: Db,
    storage: &State<Storage>,
    config: &State<Config>,
    data: Data<'_>,
) -> Result<Json<OkResponse>> {
    // Parse multipart boundary
    let boundary = content_type
        .params()
        .find(|(k, _)| *k == "boundary")
        .map(|(_, v)| v)
        .ok_or_else(|| AppError::BadRequest("Missing multipart boundary".to_string()))?;

    // Create a stream from the incoming data (up to 2GB)
    let stream = data.open(2.gibibytes());

    // Convert Rocket's DataStream to a futures Stream of bytes
    let byte_stream = tokio_util::io::ReaderStream::new(stream);
    let byte_stream = byte_stream.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));

    // Parse multipart form from stream
    let mut multipart = multer::Multipart::new(byte_stream, boundary);

    // Collect metadata fields first, stream file when we encounter it
    let mut hash: Option<String> = None;
    let mut deriver: Option<String> = None;
    let mut size: Option<i64> = None;
    let mut lock: Option<i32> = None;
    let mut drv_full: Option<String> = None;
    let mut compression: Option<String> = None;
    let mut references: Vec<String> = Vec::new();

    // Storage write result - populated when file field is processed
    let mut file_result: Option<(u64, Vec<u8>, String)> = None; // (size, hash, storage_filename)

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Multipart(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                // We need drv_full to determine the storage filename
                let storage_filename = drv_full
                    .as_ref()
                    .ok_or_else(|| {
                        AppError::BadRequest(
                            "drvFull must come before file in multipart form".to_string(),
                        )
                    })?
                    .clone();

                // Stream the file directly to storage
                // Convert multer field to AsyncRead
                let field_stream = field.map_err(|e| std::io::Error::other(e.to_string()));
                let reader = StreamReader::new(field_stream);

                // Write with hash computation - cleanup handled by storage on failure
                let (file_size, file_hash) =
                    storage.push_with_hash(&storage_filename, reader).await?;

                file_result = Some((file_size, file_hash, storage_filename));
            }
            "hash" => {
                hash = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::Multipart(e.to_string()))?,
                );
            }
            "deriver" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| AppError::Multipart(e.to_string()))?;
                if !value.is_empty() {
                    deriver = Some(value);
                }
            }
            "size" => {
                let s = field
                    .text()
                    .await
                    .map_err(|e| AppError::Multipart(e.to_string()))?;
                size = Some(
                    s.parse()
                        .map_err(|_| AppError::BadRequest("Invalid size".to_string()))?,
                );
            }
            "lock" => {
                let l = field
                    .text()
                    .await
                    .map_err(|e| AppError::Multipart(e.to_string()))?;
                lock = Some(
                    l.parse()
                        .map_err(|_| AppError::BadRequest("Invalid lock".to_string()))?,
                );
            }
            "drvFull" => {
                drv_full = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::Multipart(e.to_string()))?,
                );
            }
            "compression" => {
                compression = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::Multipart(e.to_string()))?,
                );
            }
            "references[]" | "references" => {
                let r = field
                    .text()
                    .await
                    .map_err(|e| AppError::Multipart(e.to_string()))?;
                references.push(r);
            }
            _ => {}
        }
    }

    // Validate required fields
    let (file_size, file_hash_bytes, storage_filename) =
        file_result.ok_or_else(|| AppError::BadRequest("Missing file".to_string()))?;
    let hash = hash.ok_or_else(|| AppError::BadRequest("Missing hash".to_string()))?;
    let size = size.ok_or_else(|| AppError::BadRequest("Missing size".to_string()))?;
    let lock_id = lock.ok_or_else(|| AppError::BadRequest("Missing lock".to_string()))?;
    let drv_full = drv_full.ok_or_else(|| AppError::BadRequest("Missing drvFull".to_string()))?;

    let mut conn = db.0;

    // Verify lock exists
    let lock_exists: Option<i32> = locks::table
        .select(locks::id)
        .find(lock_id)
        .first(&mut conn)
        .optional()?;

    if lock_exists.is_none() {
        // Clean up the file we just wrote since the lock is invalid
        let _ = storage.delete(&storage_filename).await;
        return Err(AppError::BadRequest("Invalid lock".to_string()));
    }

    // Extract drv_id (first part before -)
    let drv_id = drv_full
        .split('-')
        .next()
        .ok_or_else(|| AppError::BadRequest("Invalid drvFull format".to_string()))?
        .to_string();

    // Sort references
    references.sort();

    // Convert file hash to Nix format
    let file_hash_base64 = base64::prelude::BASE64_STANDARD.encode(&file_hash_bytes);
    let file_hash_nix = crate::crypto::base64_to_nix_base32(&file_hash_base64)?;
    let file_hash = format!("sha256:{}", file_hash_nix);

    // Parse NAR hash (accepts SRI or Nix format)
    let (algo, nar_hash_nix) = parse_hash(&hash)?;
    let nar_hash = format!("{}:{}", algo, nar_hash_nix);

    // Generate signature if signing key is configured
    let sig = if let Some(ref key_str) = config.signing_key {
        let signing_key = NixSigningKey::from_config(key_str)?;
        Some(signing_key.sign_drv(&drv_full, &nar_hash, size, &references))
    } else {
        None
    };

    // Determine NAR filename
    let nar_file = if let Some(ref comp) = compression {
        format!("{}.nar.{}", drv_id, comp)
    } else {
        format!("{}.nar", drv_id)
    };

    // Insert into database (delete existing first for upsert behavior)
    diesel::delete(drvs::table.find(&drv_id)).execute(&mut conn)?;

    let new_drv = NewDrv {
        drv_id: drv_id.clone(),
        drv_full,
        nar_hash,
        nar_size: size,
        file_hash,
        file_size: file_size as i64,
        deriver,
        sig,
        refs: references.into_iter().map(Some).collect(),
        nar_comp: compression,
        nar_file,
        nar_file_storage: storage_filename,
    };

    diesel::insert_into(drvs::table)
        .values(&new_drv)
        .execute(&mut conn)?;

    // Link derivation to lock
    let drv_lock = DrvLock {
        drv_id: drv_id.clone(),
        lock_id,
    };

    diesel::insert_into(drv_locks::table)
        .values(&drv_lock)
        .on_conflict_do_nothing()
        .execute(&mut conn)?;

    Ok(Json(OkResponse { ok: true }))
}
