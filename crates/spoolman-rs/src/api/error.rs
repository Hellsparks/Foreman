//! API error type that maps `SpoolmanError` to HTTP status codes.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::error::SpoolmanError;

pub struct ApiError(SpoolmanError);

impl From<SpoolmanError> for ApiError {
    fn from(e: SpoolmanError) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            SpoolmanError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            SpoolmanError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            SpoolmanError::Database(e) => {
                tracing::error!("database error: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "database error".to_owned(),
                )
            }
            SpoolmanError::Io(e) => {
                tracing::error!("I/O error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "I/O error".to_owned())
            }
            SpoolmanError::UnknownRevision(r) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("unknown alembic revision: {r}"),
            ),
        };
        (status, Json(json!({ "message": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
