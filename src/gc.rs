use chrono::Utc;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;
use std::time::Duration;
use tokio::time::interval;

use crate::schema::{drv_locks, drv_pins, drvs, locks, pins};
use crate::storage::Storage;

type DbPool = Pool<ConnectionManager<PgConnection>>;

/// Run garbage collection in a background loop
pub async fn run_gc_loop(pool: DbPool, storage: Storage) {
    let mut interval = interval(Duration::from_secs(24 * 60 * 60)); // 24 hours

    loop {
        interval.tick().await;

        tracing::info!("Starting garbage collection...");

        if let Err(e) = run_gc(&pool, &storage).await {
            tracing::error!("Garbage collection failed: {}", e);
        }
    }
}

/// Run a single garbage collection cycle
pub async fn run_gc(pool: &DbPool, storage: &Storage) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut conn = pool.get()?;

    // Step 1: Clear expired locks
    tracing::info!("GC: Clearing expired locks...");
    let now = Utc::now().naive_utc();

    let deleted_locks = diesel::delete(locks::table.filter(locks::expires.lt(now)))
        .execute(&mut conn)?;

    tracing::info!("GC: Deleted {} expired locks", deleted_locks);

    // Step 2: Clear expired abandoned pins
    tracing::info!("GC: Clearing expired abandoned pins...");

    let deleted_pins = diesel::delete(
        pins::table
            .filter(pins::abandoned.eq(true))
            .filter(pins::expires.lt(now)),
    )
    .execute(&mut conn)?;

    tracing::info!("GC: Deleted {} expired pins", deleted_pins);

    // Step 3: Find and delete unreferenced derivations
    tracing::info!("GC: Finding unreferenced derivations...");

    loop {
        // Find derivations with no pins or locks
        // This is done one at a time to handle cascading deletes properly
        let orphan: Option<(String, String, String)> = drvs::table
            .left_join(drv_pins::table.on(drvs::drv_id.eq(drv_pins::drv_id)))
            .left_join(drv_locks::table.on(drvs::drv_id.eq(drv_locks::drv_id)))
            .group_by(drvs::drv_id)
            .having(
                diesel::dsl::sql::<diesel::sql_types::Bool>(
                    "COUNT(drv_pins.drv_id) + COUNT(drv_locks.drv_id) = 0"
                )
            )
            .select((drvs::drv_id, drvs::drv_full, drvs::nar_file_storage))
            .first(&mut conn)
            .optional()?;

        match orphan {
            Some((drv_id, drv_full, nar_file_storage)) => {
                tracing::info!("GC: Removing {}...", drv_full);

                // Set GC flag
                diesel::update(drvs::table.find(&drv_id))
                    .set(drvs::gc.eq(true))
                    .execute(&mut conn)?;

                // Delete from storage
                if let Err(e) = storage.delete(&nar_file_storage).await {
                    tracing::warn!("GC: Failed to delete {} from storage: {}", nar_file_storage, e);
                }

                // Delete from database
                diesel::delete(drvs::table.find(&drv_id)).execute(&mut conn)?;
            }
            None => {
                // No more orphans
                break;
            }
        }
    }

    tracing::info!("GC: Completed");

    Ok(())
}

/// Run initial GC on startup (optional)
pub async fn run_initial_gc(pool: &DbPool, storage: &Storage) {
    tracing::info!("Running initial garbage collection...");

    if let Err(e) = run_gc(pool, storage).await {
        tracing::warn!("Initial garbage collection failed: {}", e);
    }
}
