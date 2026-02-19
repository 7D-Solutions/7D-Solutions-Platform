//! External ID registry domain: maps internal entities to external system IDs.

pub mod guards;
pub mod models;
pub mod service;

pub use models::{
    CreateExternalRefRequest, ExternalRef, ExternalRefError, UpdateExternalRefRequest,
};
