use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpoolmanError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("configuration error: {0}")]
    Config(String),

    #[error(
        "unknown or newer alembic revision '{0}' — refusing to start; \
         run the upstream Python Spoolman migration first, or point to a fresh database"
    )]
    UnknownRevision(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not found: {0}")]
    NotFound(String),

    /// 400-class client error (bad parameters, etc.)
    #[error("{0}")]
    BadRequest(String),

    /// 422-class error: entity validation failure (unknown extra field keys, etc.)
    #[error("{0}")]
    ValidationError(String),

    /// 422-class error for measure operations (initial weight unknown, etc.)
    #[error("{0}")]
    MeasureError(String),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// 403 — entity cannot be deleted because it is referenced by another entity.
    #[error("{0}")]
    DeleteConflict(String),

    /// 409 conflict — tag UID already linked to a different entity.
    #[error("{message}")]
    TagConflict {
        message: String,
        spool_id: Option<i64>,
        filament_id: Option<i64>,
    },
}

pub type Result<T> = std::result::Result<T, SpoolmanError>;
