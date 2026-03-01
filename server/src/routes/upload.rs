use base64::Engine;
use diesel::prelude::*;
use rocket::data::{Data, ToByteUnit};
use rocket::http::ContentType;
use rocket::put;
use rocket::serde::json::Json;
use rocket::State;
use sha2::{Digest, Sha256};
use std::io::Write;

use crate::auth::AuthenticatedUser;
use crate::config::Config;
use crate::crypto::{sri_to_nix_hash, NixSigningKey};
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::{DrvLock, NewDrv, OkResponse};
use crate::schema::{drv_locks, drvs, locks};
use crate::storage::Storage;

/// PUT /uploadNar
/// Upload a NAR file with metadata (multipart form)
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

    // Read the entire data (up to 2GB)
    let stream = data.open(2.gibibytes());
    let bytes = stream
        .into_bytes()
        .await
        .map_err(|e| AppError::Io(e.into()))?;

    // Parse multipart form
    let mut multipart = multer::Multipart::new(
        futures::stream::once(async move { Ok::<_, std::io::Error>(bytes.value) }),
        boundary,
    );

    let mut file_data: Option<Vec<u8>> = None;
    let mut hash: Option<String> = None;
    let mut deriver: Option<String> = None;
    let mut size: Option<i64> = None;
    let mut lock: Option<i32> = None;
    let mut drv_full: Option<String> = None;
    let mut compression: Option<String> = None;
    let mut references: Vec<String> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Multipart(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();

        match name.as_str() {
            "file" => {
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::Multipart(e.to_string()))?
                        .to_vec(),
                );
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
                deriver = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::Multipart(e.to_string()))?,
                );
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
    let file_data = file_data.ok_or_else(|| AppError::BadRequest("Missing file".to_string()))?;
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

    // Compute file hash (SHA256 of compressed file)
    let mut hasher = Sha256::new();
    hasher.update(&file_data);
    let file_hash_bytes = hasher.finalize();
    let file_hash_base64 = base64::prelude::BASE64_STANDARD.encode(&file_hash_bytes);
    let file_hash_nix = crate::crypto::base64_to_nix_base32(&file_hash_base64)?;
    let file_hash = format!("sha256:{}", file_hash_nix);

    let file_size = file_data.len() as i64;

    // Convert NAR hash from SRI format to Nix format
    let (algo, nar_hash_nix) = sri_to_nix_hash(&hash)?;
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

    // Write file to storage
    let storage_filename = drv_full.clone();
    let mut file = std::fs::File::create(storage.base_path.join(&storage_filename))?;
    file.write_all(&file_data)?;

    // Insert into database (delete existing first for upsert behavior)
    diesel::delete(drvs::table.find(&drv_id)).execute(&mut conn)?;

    let new_drv = NewDrv {
        drv_id: drv_id.clone(),
        drv_full,
        nar_hash,
        nar_size: size,
        file_hash,
        file_size,
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
