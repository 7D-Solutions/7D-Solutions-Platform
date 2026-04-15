//! Branding images domain model.
//!
//! Invariants:
//! - Every image is tenant-scoped; all queries filter by tenant_id
//! - Supported formats: PNG, JPEG, SVG only
//! - Maximum image size: 5 MB
//! - Idempotent upload via (tenant_id, idempotency_key) unique constraint
//! - Placement must be one of: header_logo, footer_branding, inline

pub mod repo;

pub use repo::ImageRepo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Maximum image file size (5 MB).
pub const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;

const SUPPORTED_FORMATS: &[&str] = &["png", "jpeg", "svg"];
const VALID_PLACEMENTS: &[&str] = &["header_logo", "footer_branding", "inline"];

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BrandingImage {
    pub id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub name: String,
    pub image_format: String,
    pub width_px: Option<i32>,
    pub height_px: Option<i32>,
    pub size_bytes: i64,
    pub image_data: Vec<u8>,
    pub placement: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug)]
pub struct UploadImageRequest {
    pub tenant_id: String,
    pub idempotency_key: String,
    pub name: String,
    pub image_format: String,
    pub image_data: Vec<u8>,
    pub placement: String,
    pub created_by: String,
}

#[derive(Debug, Deserialize)]
pub struct ListImagesQuery {
    pub tenant_id: String,
    pub placement: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Event payload
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ImageUploadedPayload {
    pub tenant_id: String,
    pub image_id: Uuid,
    pub name: String,
    pub image_format: String,
    pub placement: String,
    pub size_bytes: i64,
}

// ============================================================================
// Validation (Guard)
// ============================================================================

pub fn validate_upload(req: &UploadImageRequest) -> Result<(), ImageError> {
    if req.tenant_id.trim().is_empty() {
        return Err(ImageError::Validation("tenant_id is required".into()));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(ImageError::Validation("idempotency_key is required".into()));
    }
    if req.name.trim().is_empty() {
        return Err(ImageError::Validation("name is required".into()));
    }
    if req.created_by.trim().is_empty() {
        return Err(ImageError::Validation("created_by is required".into()));
    }
    if !SUPPORTED_FORMATS.contains(&req.image_format.as_str()) {
        return Err(ImageError::UnsupportedFormat(req.image_format.clone()));
    }
    if !VALID_PLACEMENTS.contains(&req.placement.as_str()) {
        return Err(ImageError::Validation(format!(
            "invalid placement '{}', must be one of: {}",
            req.placement,
            VALID_PLACEMENTS.join(", ")
        )));
    }
    if req.image_data.is_empty() {
        return Err(ImageError::Validation("image_data is empty".into()));
    }
    if req.image_data.len() > MAX_IMAGE_SIZE {
        return Err(ImageError::TooLarge);
    }
    Ok(())
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("Image exceeds maximum size of {MAX_IMAGE_SIZE} bytes")]
    TooLarge,

    #[error("Unsupported image format: {0}")]
    UnsupportedFormat(String),

    #[error("Image not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
