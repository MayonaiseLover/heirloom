use thiserror::Error;

/// Result type for heirloom-core operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by the core engine.
#[derive(Error, Debug)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("memory not found: {0}")]
    NotFound(String),

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    #[error("{0}")]
    Other(String),
}
