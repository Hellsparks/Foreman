use thiserror::Error;

/// Top-level application error type.
///
/// Every public function that can fail returns `Result<T, AppError>`.
/// No `unwrap()` / `expect()` in server code paths.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("internal error: {0}")]
    Internal(String),
}
