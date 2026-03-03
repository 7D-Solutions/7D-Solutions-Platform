//! Image repository — database access layer.
//!
//! All mutations follow Guard → Mutation → Outbox:
//! 1. Guard: validate_upload() checks format, size, tenant_id
//! 2. Mutation: INSERT into branding_images (idempotent via unique constraint)
//! 3. Outbox: enqueue pdf.image.uploaded event in same transaction

use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_pdf_editor_envelope, enqueue_event};

use super::{
    validate_upload, BrandingImage, ImageError, ImageUploadedPayload, ListImagesQuery,
    UploadImageRequest,
};

pub struct ImageRepo;

impl ImageRepo {
    /// Upload a branding image. Idempotent: duplicate (tenant_id, idempotency_key)
    /// returns the existing image without creating a new outbox event.
    ///
    /// Flow: Guard → Mutation → Outbox (all in one transaction).
    pub async fn upload(
        pool: &PgPool,
        req: &UploadImageRequest,
    ) -> Result<BrandingImage, ImageError> {
        // ── Guard ──────────────────────────────────────────────
        validate_upload(req)?;

        // Detect dimensions for raster formats (PNG, JPEG)
        let (width_px, height_px) = detect_dimensions(&req.image_format, &req.image_data);
        let size_bytes = req.image_data.len() as i64;

        // ── Check idempotency (return existing if duplicate key) ──
        let existing: Option<BrandingImage> = sqlx::query_as(
            "SELECT * FROM branding_images WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(req.tenant_id.trim())
        .bind(req.idempotency_key.trim())
        .fetch_optional(pool)
        .await?;

        if let Some(img) = existing {
            return Ok(img);
        }

        // ── Mutation + Outbox (single transaction) ─────────────
        let mut tx = pool.begin().await?;

        let image: BrandingImage = sqlx::query_as(
            r#"
            INSERT INTO branding_images
                (tenant_id, idempotency_key, name, image_format,
                 width_px, height_px, size_bytes, image_data, placement, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING *
            "#,
        )
        .bind(req.tenant_id.trim())
        .bind(req.idempotency_key.trim())
        .bind(req.name.trim())
        .bind(&req.image_format)
        .bind(width_px)
        .bind(height_px)
        .bind(size_bytes)
        .bind(&req.image_data)
        .bind(&req.placement)
        .bind(req.created_by.trim())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ─────────────────────────────────────────────
        let payload = ImageUploadedPayload {
            tenant_id: image.tenant_id.clone(),
            image_id: image.id,
            name: image.name.clone(),
            image_format: image.image_format.clone(),
            placement: image.placement.clone(),
            size_bytes: image.size_bytes,
        };
        let envelope = create_pdf_editor_envelope(
            Uuid::new_v4(),
            image.tenant_id.clone(),
            "pdf.image.uploaded".to_string(),
            None,
            None,
            "DATA_MUTATION".to_string(),
            payload,
        );
        enqueue_event(&mut tx, "pdf.image.uploaded", &envelope).await?;

        tx.commit().await?;
        Ok(image)
    }

    /// Find an image by ID with tenant isolation.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<BrandingImage>, ImageError> {
        sqlx::query_as::<_, BrandingImage>(
            "SELECT * FROM branding_images WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(ImageError::Database)
    }

    /// List images for a tenant, optionally filtered by placement.
    pub async fn list(
        pool: &PgPool,
        q: &ListImagesQuery,
    ) -> Result<Vec<BrandingImage>, ImageError> {
        if q.tenant_id.trim().is_empty() {
            return Err(ImageError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).clamp(1, 100);
        let offset = q.offset.unwrap_or(0);

        sqlx::query_as::<_, BrandingImage>(
            r#"
            SELECT * FROM branding_images
            WHERE tenant_id = $1
              AND ($2::text IS NULL OR placement = $2)
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(q.tenant_id.trim())
        .bind(q.placement.as_deref())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(ImageError::Database)
    }
}

/// Detect width/height for raster image formats.
/// Returns (None, None) for SVG since dimensions are in the XML markup.
fn detect_dimensions(format: &str, data: &[u8]) -> (Option<i32>, Option<i32>) {
    if format == "svg" {
        return (None, None);
    }
    match image::load_from_memory(data) {
        Ok(img) => (Some(img.width() as i32), Some(img.height() as i32)),
        Err(_) => (None, None),
    }
}
