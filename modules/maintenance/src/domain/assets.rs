//! Maintainable asset domain model and repository.
//!
//! Invariants:
//! - asset_tag is unique per tenant (DB constraint + application guard)
//! - tenant_id, asset_tag, name, asset_type are required and non-empty
//! - Every query filters by tenant_id for multi-tenant isolation
//! - All mutations use Guard → Mutation → Outbox atomicity
//! - idempotency_key prevents duplicate asset creation per tenant

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::work_orders::types::{AssetStatus, AssetType};
use crate::events::{envelope, subjects};
use crate::outbox;

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Asset {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_tag: String,
    pub name: String,
    pub description: Option<String>,
    #[sqlx(try_from = "String")]
    pub asset_type: AssetType,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub serial_number: Option<String>,
    pub fixed_asset_ref: Option<Uuid>,
    #[sqlx(try_from = "String")]
    pub status: AssetStatus,
    pub metadata: Option<serde_json::Value>,
    pub maintenance_schedule: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub out_of_service: bool,
    pub out_of_service_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAssetRequest {
    pub tenant_id: String,
    pub asset_tag: String,
    pub name: String,
    pub description: Option<String>,
    pub asset_type: String,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub serial_number: Option<String>,
    pub fixed_asset_ref: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub maintenance_schedule: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAssetRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub asset_type: Option<String>,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub serial_number: Option<String>,
    pub fixed_asset_ref: Option<Uuid>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub maintenance_schedule: Option<serde_json::Value>,
    pub out_of_service: Option<bool>,
    pub out_of_service_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub tenant_id: String,
    pub asset_type: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AssetListResponse {
    pub items: Vec<Asset>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("Asset tag '{0}' already exists for tenant '{1}'")]
    DuplicateTag(String, String),

    #[error("Asset not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent duplicate — returning existing asset")]
    IdempotentDuplicate(Box<Asset>),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct AssetRepo;

impl AssetRepo {
    /// Create an asset using Guard → Mutation → Outbox.
    pub async fn create(pool: &PgPool, req: &CreateAssetRequest) -> Result<Asset, AssetError> {
        // ── Guard ──
        if req.tenant_id.trim().is_empty() {
            return Err(AssetError::Validation("tenant_id is required".into()));
        }
        if req.asset_tag.trim().is_empty() {
            return Err(AssetError::Validation("asset_tag is required".into()));
        }
        if req.name.trim().is_empty() {
            return Err(AssetError::Validation("name is required".into()));
        }
        AssetType::from_str_value(&req.asset_type)
            .map_err(|e| AssetError::Validation(e.to_string()))?;

        // ── Idempotency check ──
        if let Some(ref ikey) = req.idempotency_key {
            let existing = sqlx::query_as::<_, Asset>(
                "SELECT * FROM maintainable_assets WHERE tenant_id = $1 AND idempotency_key = $2",
            )
            .bind(&req.tenant_id)
            .bind(ikey)
            .fetch_optional(pool)
            .await?;

            if let Some(asset) = existing {
                return Err(AssetError::IdempotentDuplicate(Box::new(asset)));
            }
        }

        let mut tx = pool.begin().await?;

        // ── Mutation ──
        let id = Uuid::new_v4();
        let now = Utc::now();

        let asset = sqlx::query_as::<_, Asset>(
            r#"
            INSERT INTO maintainable_assets
                (id, tenant_id, asset_tag, name, description, asset_type,
                 location, department, responsible_person, serial_number,
                 fixed_asset_ref, metadata, maintenance_schedule, idempotency_key,
                 created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.asset_tag.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(&req.asset_type)
        .bind(req.location.as_deref())
        .bind(req.department.as_deref())
        .bind(req.responsible_person.as_deref())
        .bind(req.serial_number.as_deref())
        .bind(req.fixed_asset_ref)
        .bind(&req.metadata)
        .bind(&req.maintenance_schedule)
        .bind(req.idempotency_key.as_deref())
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return AssetError::DuplicateTag(req.asset_tag.clone(), req.tenant_id.clone());
                }
            }
            AssetError::Database(e)
        })?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "asset_id": id,
            "tenant_id": &req.tenant_id,
            "asset_tag": req.asset_tag.trim(),
            "name": req.name.trim(),
            "asset_type": &req.asset_type,
            "serial_number": req.serial_number,
            "location": req.location,
            "status": "active",
        });
        let event_id = Uuid::new_v4();
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::ASSET_CREATED.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| AssetError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::ASSET_CREATED,
            "asset",
            &id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(asset)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Asset>, AssetError> {
        sqlx::query_as::<_, Asset>(
            "SELECT * FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(AssetError::Database)
    }

    pub async fn list(pool: &PgPool, q: &ListAssetsQuery) -> Result<AssetListResponse, AssetError> {
        if q.tenant_id.trim().is_empty() {
            return Err(AssetError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).clamp(1, 100);
        let offset = q.offset.unwrap_or(0);

        // Validate optional filter values
        if let Some(ref at) = q.asset_type {
            AssetType::from_str_value(at).map_err(|e| AssetError::Validation(e.to_string()))?;
        }
        if let Some(ref st) = q.status {
            AssetStatus::from_str_value(st).map_err(|e| AssetError::Validation(e.to_string()))?;
        }

        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM maintainable_assets
            WHERE tenant_id = $1
              AND ($2::TEXT IS NULL OR asset_type = $2)
              AND ($3::TEXT IS NULL OR status = $3)
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.asset_type.as_deref())
        .bind(q.status.as_deref())
        .fetch_one(pool)
        .await?;

        let items = sqlx::query_as::<_, Asset>(
            r#"
            SELECT * FROM maintainable_assets
            WHERE tenant_id = $1
              AND ($2::TEXT IS NULL OR asset_type = $2)
              AND ($3::TEXT IS NULL OR status = $3)
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.asset_type.as_deref())
        .bind(q.status.as_deref())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(AssetListResponse {
            items,
            total: total.0,
            limit,
            offset,
        })
    }

    /// Update an asset using Guard → Mutation → Outbox.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        req: &UpdateAssetRequest,
    ) -> Result<Asset, AssetError> {
        // ── Guard ──
        if let Some(ref at) = req.asset_type {
            AssetType::from_str_value(at).map_err(|e| AssetError::Validation(e.to_string()))?;
        }
        if let Some(ref st) = req.status {
            AssetStatus::from_str_value(st).map_err(|e| AssetError::Validation(e.to_string()))?;
        }
        if let Some(ref name) = req.name {
            if name.trim().is_empty() {
                return Err(AssetError::Validation("name must not be empty".into()));
            }
        }

        let mut tx = pool.begin().await?;

        // Fetch current state to detect out_of_service changes
        let before: Option<(bool,)> = sqlx::query_as(
            "SELECT out_of_service FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
        let before_oos = before.map(|b| b.0);

        // ── Mutation ──
        let asset = sqlx::query_as::<_, Asset>(
            r#"
            UPDATE maintainable_assets SET
                name                  = COALESCE($3, name),
                description           = COALESCE($4, description),
                asset_type            = COALESCE($5, asset_type),
                location              = COALESCE($6, location),
                department            = COALESCE($7, department),
                responsible_person    = COALESCE($8, responsible_person),
                serial_number         = COALESCE($9, serial_number),
                fixed_asset_ref       = COALESCE($10, fixed_asset_ref),
                status                = COALESCE($11, status),
                metadata              = COALESCE($12, metadata),
                maintenance_schedule  = COALESCE($13, maintenance_schedule),
                out_of_service        = COALESCE($14, out_of_service),
                out_of_service_reason = CASE
                    WHEN $14 = TRUE THEN COALESCE($15, out_of_service_reason)
                    WHEN $14 = FALSE THEN NULL
                    ELSE COALESCE($15, out_of_service_reason)
                END,
                updated_at            = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.asset_type.as_deref())
        .bind(req.location.as_deref())
        .bind(req.department.as_deref())
        .bind(req.responsible_person.as_deref())
        .bind(req.serial_number.as_deref())
        .bind(req.fixed_asset_ref)
        .bind(req.status.as_deref())
        .bind(&req.metadata)
        .bind(&req.maintenance_schedule)
        .bind(req.out_of_service)
        .bind(req.out_of_service_reason.as_deref())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AssetError::NotFound)?;

        // ── Outbox: asset.updated ──
        let event_payload = serde_json::json!({
            "asset_id": id,
            "tenant_id": tenant_id,
            "status": asset.status.as_str(),
        });
        let event_id = Uuid::new_v4();
        let env = envelope::create_envelope(
            event_id,
            tenant_id.to_string(),
            subjects::ASSET_UPDATED.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| AssetError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::ASSET_UPDATED,
            "asset",
            &id.to_string(),
            &env_json,
        )
        .await?;

        // ── Outbox: out_of_service_changed (only if changed) ──
        if let Some(before_val) = before_oos {
            if req.out_of_service.is_some() && asset.out_of_service != before_val {
                let oos_payload = serde_json::json!({
                    "asset_id": id,
                    "tenant_id": tenant_id,
                    "out_of_service": asset.out_of_service,
                    "out_of_service_reason": asset.out_of_service_reason,
                });
                let oos_event_id = Uuid::new_v4();
                let oos_env = envelope::create_envelope(
                    oos_event_id,
                    tenant_id.to_string(),
                    subjects::OUT_OF_SERVICE_CHANGED.to_string(),
                    oos_payload,
                );
                let oos_env_json = envelope::validate_envelope(&oos_env)
                    .map_err(|e| AssetError::Validation(format!("envelope: {}", e)))?;
                outbox::enqueue_event_tx(
                    &mut tx,
                    oos_event_id,
                    subjects::OUT_OF_SERVICE_CHANGED,
                    "asset",
                    &id.to_string(),
                    &oos_env_json,
                )
                .await?;
            }
        }

        tx.commit().await?;
        Ok(asset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_type_roundtrip() {
        assert_eq!(
            AssetType::from_str_value("vehicle").unwrap(),
            AssetType::Vehicle
        );
        assert_eq!(
            AssetType::from_str_value("facility").unwrap(),
            AssetType::Facility
        );
        assert!(AssetType::from_str_value("spaceship").is_err());
    }

    #[test]
    fn asset_status_roundtrip() {
        assert_eq!(
            AssetStatus::from_str_value("active").unwrap(),
            AssetStatus::Active
        );
        assert_eq!(
            AssetStatus::from_str_value("retired").unwrap(),
            AssetStatus::Retired
        );
        assert!(AssetStatus::from_str_value("destroyed").is_err());
    }
}
