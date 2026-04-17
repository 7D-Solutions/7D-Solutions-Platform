mod headers;
mod lines;

use thiserror::Error;

use crate::domain::guards::GuardError;

pub use crate::domain::bom_queries::{explode, where_used};
pub use headers::*;
pub use lines::*;

#[derive(Debug, Error)]
pub enum BomError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Inventory service unavailable: {0}")]
    InventoryUnavailable(String),
}
