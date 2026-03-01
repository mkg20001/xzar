use rocket::http::Status;
use rocket::response::{self, Responder};
use rocket::serde::json::Json;
use rocket::Request;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] diesel::result::Error),

    #[error("Database connection error: {0}")]
    DatabaseConnection(#[from] diesel::r2d2::PoolError),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Crypto error: {0}")]
    Crypto(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Lock not found or expired")]
    LockNotFound,

    #[error("Multipart error: {0}")]
    Multipart(String),
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl<'r> Responder<'r, 'static> for AppError {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (Status::NotFound, msg.clone()),
            AppError::Unauthorized => (Status::Unauthorized, "Unauthorized".to_string()),
            AppError::BadRequest(msg) => (Status::BadRequest, msg.clone()),
            AppError::LockNotFound => (Status::NotFound, "Lock not found or expired".to_string()),
            AppError::Database(e) => {
                tracing::error!("Database error: {}", e);
                (Status::InternalServerError, "Database error".to_string())
            }
            AppError::DatabaseConnection(e) => {
                tracing::error!("Database connection error: {}", e);
                (
                    Status::InternalServerError,
                    "Database connection error".to_string(),
                )
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (Status::InternalServerError, "IO error".to_string())
            }
            AppError::Crypto(msg) => {
                tracing::error!("Crypto error: {}", msg);
                (Status::InternalServerError, "Crypto error".to_string())
            }
            AppError::Config(msg) => {
                tracing::error!("Config error: {}", msg);
                (Status::InternalServerError, "Config error".to_string())
            }
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (Status::InternalServerError, msg.clone())
            }
            AppError::Multipart(msg) => (Status::BadRequest, msg.clone()),
        };

        let response = ErrorResponse {
            error: status.reason().unwrap_or("Error").to_string(),
            message: Some(message),
        };

        Json(response).respond_to(req).map(|mut r| {
            r.set_status(status);
            r
        })
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
