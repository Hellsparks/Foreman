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

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError(SpoolmanError::Json(e))
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError(SpoolmanError::Io(e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if let SpoolmanError::TagConflict {
            message,
            spool_id,
            filament_id,
        } = &self.0
        {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "message": message,
                    "spool_id": spool_id,
                    "filament_id": filament_id,
                })),
            )
                .into_response();
        }

        let (status, message) = match &self.0 {
            SpoolmanError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            SpoolmanError::DeleteConflict(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            SpoolmanError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            SpoolmanError::ValidationError(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
            SpoolmanError::MeasureError(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
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
            SpoolmanError::Json(e) => {
                tracing::error!("JSON error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "JSON error".to_owned())
            }
            SpoolmanError::TagConflict { .. } => unreachable!("handled above"),
        };
        (status, Json(json!({ "message": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
