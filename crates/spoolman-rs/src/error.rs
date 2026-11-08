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
}

pub type Result<T> = std::result::Result<T, SpoolmanError>;
