use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{drv_locks, drv_pins, drvs, locks, pins};

// ============ Derivations ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = drvs)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Drv {
    pub drv_id: String,
    pub drv_full: String,
    pub nar_hash: String,
    pub nar_size: i64,
    pub file_hash: String,
    pub file_size: i64,
    pub deriver: Option<String>,
    pub sig: Option<String>,
    pub refs: Vec<Option<String>>,
    pub nar_comp: Option<String>,
    pub nar_file: String,
    pub nar_file_storage: String,
    pub gc: bool,
    pub created: NaiveDateTime,
    pub last_fetched: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = drvs)]
pub struct NewDrv {
    pub drv_id: String,
    pub drv_full: String,
    pub nar_hash: String,
    pub nar_size: i64,
    pub file_hash: String,
    pub file_size: i64,
    pub deriver: Option<String>,
    pub sig: Option<String>,
    pub refs: Vec<Option<String>>,
    pub nar_comp: Option<String>,
    pub nar_file: String,
    pub nar_file_storage: String,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = drvs)]
pub struct UpdateDrv {
    pub last_fetched: Option<NaiveDateTime>,
    pub gc: Option<bool>,
}

// ============ Locks ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = locks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Lock {
    pub id: i32,
    pub expires: NaiveDateTime,
    pub owner: i32,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = locks)]
pub struct NewLock {
    pub owner: i32,
}

// ============ Pins ============

#[derive(Debug, Clone, Queryable, Selectable, Serialize)]
#[diesel(table_name = pins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pin {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created: NaiveDateTime,
    pub expires: Option<NaiveDateTime>,
    pub abandoned: bool,
    pub leave_after_abandon: Option<i64>,
}

#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = pins)]
pub struct NewPin {
    pub name: String,
    pub description: Option<String>,
    pub expires: Option<NaiveDateTime>,
    pub leave_after_abandon: Option<i64>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = pins)]
pub struct UpdatePin {
    pub abandoned: Option<bool>,
    pub expires: Option<Option<NaiveDateTime>>,
}

// ============ Junction Tables ============

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = drv_locks)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DrvLock {
    pub drv_id: String,
    pub lock_id: i32,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = drv_pins)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DrvPin {
    pub drv_id: String,
    pub pin_id: i32,
}

// ============ Request/Response DTOs ============

#[derive(Debug, Deserialize)]
pub struct CheckRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CheckResponse {
    pub need: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LockResponse {
    pub lock: i32,
    pub deadline: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct LockExtendRequest {
    pub lock: i32,
}

#[derive(Debug, Deserialize)]
pub struct LockClearRequest {
    pub lock: i32,
}

#[derive(Debug, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

#[derive(Debug, Deserialize)]
pub struct FinalizePinRequest {
    pub roots: Vec<String>,
    pub name: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub expires: Option<i64>,
    #[serde(rename = "leaveAfterAbandon", default)]
    pub leave_after_abandon: Option<i64>,
}
