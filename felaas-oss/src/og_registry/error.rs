//! Error types for the OG registry module.

use thiserror::Error;

use crate::common::ChatUserId;

/// Error type for OG registry operations.
#[derive(Debug, Error)]
pub enum OgRegistryError {
    /// Another OG with the same reference_user_id is already active
    #[error("Another OG with reference_user_id {reference_user_id} is already active (existing user_id: {existing_user_id})")]
    DuplicateActiveOg {
        reference_user_id: ChatUserId,
        existing_user_id: ChatUserId,
    },

    /// The requested OG was not found
    #[error("OG not found: {0}")]
    OgNotFound(ChatUserId),

    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(#[from] tokio_postgres::Error),

    /// Error converting row data
    #[error("Row conversion error: {0}")]
    RowConversionError(String),

    /// Database pool error
    #[error("Database pool error: {0}")]
    PoolError(#[from] deadpool_postgres::PoolError),

    /// Other unexpected errors
    #[error("Unexpected error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Result type alias for OG registry operations
pub type Result<T> = std::result::Result<T, OgRegistryError>;
