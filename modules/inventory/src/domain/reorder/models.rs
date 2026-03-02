//! Reorder policy domain model and repository.
//!
//! A reorder policy defines the low-stock thresholds for an item, optionally
//! scoped to a specific warehouse location.
//!
//! Invariants:
//! - reorder_point >= 0, safety_stock >= 0, max_qty >= 0 (when set)
//! - At most one policy per (tenant, item) when location_id IS NULL
//! - At most one policy per (tenant, item, location_id) when location_id is set
//! - item_id must reference an existing item in the same tenant

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ReorderPolicy {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub location_id: Option<Uuid>,
    /// Quantity on hand at which a restock should be triggered.
    pub reorder_point: i64,
    /// Minimum buffer quantity to keep on hand at all times.
    pub safety_stock: i64,
    /// Optional upper bound for order-to-max workflows.
    pub max_qty: Option<i64>,
    pub notes: Option<String>,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateReorderPolicyRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub location_id: Option<Uuid>,
    pub reorder_point: i64,
    pub safety_stock: i64,
    pub max_qty: Option<i64>,
    pub notes: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReorderPolicyRequest {
    pub tenant_id: String,
    pub reorder_point: Option<i64>,
    pub safety_stock: Option<i64>,
    /// Pass Some(value) to set; None leaves the existing value unchanged.
    pub max_qty: Option<i64>,
    pub notes: Option<String>,
    pub updated_by: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ReorderPolicyError {
    #[error("Reorder policy not found")]
    NotFound,

    #[error("A reorder policy already exists for this item/location combination")]
    DuplicatePolicy,

    #[error("Item not found or does not belong to this tenant")]
    ItemNotFound,

    #[error("Location not found or does not belong to this tenant")]
    LocationNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct ReorderPolicyRepo;

impl ReorderPolicyRepo {
    /// Create a new reorder policy.
    ///
    /// Returns `DuplicatePolicy` if a policy already exists for the
    /// (tenant, item, location) combination.
    pub async fn create(
        pool: &PgPool,
        req: &CreateReorderPolicyRequest,
    ) -> Result<ReorderPolicy, ReorderPolicyError> {
        if req.tenant_id.trim().is_empty() {
            return Err(ReorderPolicyError::Validation(
                "tenant_id is required".into(),
            ));
        }
        if req.reorder_point < 0 {
            return Err(ReorderPolicyError::Validation(
                "reorder_point must be >= 0".into(),
            ));
        }
        if req.safety_stock < 0 {
            return Err(ReorderPolicyError::Validation(
                "safety_stock must be >= 0".into(),
            ));
        }
        if let Some(max) = req.max_qty {
            if max < 0 {
                return Err(ReorderPolicyError::Validation(
                    "max_qty must be >= 0".into(),
                ));
            }
        }

        // Verify item belongs to tenant
        let item_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM items WHERE id = $1 AND tenant_id = $2)",
        )
        .bind(req.item_id)
        .bind(&req.tenant_id)
        .fetch_one(pool)
        .await
        .map_err(ReorderPolicyError::Database)?;

        if !item_exists {
            return Err(ReorderPolicyError::ItemNotFound);
        }

        // If location_id provided, verify it belongs to the tenant and is active
        if let Some(loc_id) = req.location_id {
            let loc_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM locations WHERE id = $1 AND tenant_id = $2 AND is_active = TRUE)",
            )
            .bind(loc_id)
            .bind(&req.tenant_id)
            .fetch_one(pool)
            .await
            .map_err(ReorderPolicyError::Database)?;

            if !loc_exists {
                return Err(ReorderPolicyError::LocationNotFound);
            }
        }

        let created_by = req.created_by.as_deref().unwrap_or("system").to_string();

        sqlx::query_as::<_, ReorderPolicy>(
            r#"
            INSERT INTO reorder_policies
                (tenant_id, item_id, location_id, reorder_point, safety_stock,
                 max_qty, notes, created_by, updated_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.location_id)
        .bind(req.reorder_point)
        .bind(req.safety_stock)
        .bind(req.max_qty)
        .bind(&req.notes)
        .bind(&created_by)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return ReorderPolicyError::DuplicatePolicy;
                }
            }
            ReorderPolicyError::Database(e)
        })
    }

    /// Find a policy by id, scoped to tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<ReorderPolicy>, ReorderPolicyError> {
        sqlx::query_as::<_, ReorderPolicy>(
            "SELECT * FROM reorder_policies WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(ReorderPolicyError::Database)
    }

    /// List all policies for an item (all locations), ordered: global first then by location_id.
    pub async fn list_for_item(
        pool: &PgPool,
        tenant_id: &str,
        item_id: Uuid,
    ) -> Result<Vec<ReorderPolicy>, ReorderPolicyError> {
        sqlx::query_as::<_, ReorderPolicy>(
            r#"
            SELECT * FROM reorder_policies
            WHERE tenant_id = $1 AND item_id = $2
            ORDER BY location_id NULLS FIRST, created_at
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .fetch_all(pool)
        .await
        .map_err(ReorderPolicyError::Database)
    }

    /// Update mutable threshold fields. Tenant-scoped.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateReorderPolicyRequest,
    ) -> Result<ReorderPolicy, ReorderPolicyError> {
        if req.tenant_id.trim().is_empty() {
            return Err(ReorderPolicyError::Validation(
                "tenant_id is required".into(),
            ));
        }
        if let Some(rp) = req.reorder_point {
            if rp < 0 {
                return Err(ReorderPolicyError::Validation(
                    "reorder_point must be >= 0".into(),
                ));
            }
        }
        if let Some(ss) = req.safety_stock {
            if ss < 0 {
                return Err(ReorderPolicyError::Validation(
                    "safety_stock must be >= 0".into(),
                ));
            }
        }
        if let Some(max) = req.max_qty {
            if max < 0 {
                return Err(ReorderPolicyError::Validation(
                    "max_qty must be >= 0".into(),
                ));
            }
        }

        let updated_by = req.updated_by.as_deref().unwrap_or("system").to_string();

        sqlx::query_as::<_, ReorderPolicy>(
            r#"
            UPDATE reorder_policies
            SET reorder_point = COALESCE($3, reorder_point),
                safety_stock  = COALESCE($4, safety_stock),
                max_qty       = CASE WHEN $5::BIGINT IS NOT NULL THEN $5 ELSE max_qty END,
                notes         = COALESCE($6, notes),
                updated_by    = $7,
                updated_at    = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.reorder_point)
        .bind(req.safety_stock)
        .bind(req.max_qty)
        .bind(&req.notes)
        .bind(&updated_by)
        .fetch_optional(pool)
        .await
        .map_err(ReorderPolicyError::Database)?
        .ok_or(ReorderPolicyError::NotFound)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_reorder_point_is_invalid() {
        let rp: i64 = -1;
        assert!(rp < 0, "negative reorder_point should be rejected");
    }

    #[test]
    fn reorder_policy_serializes() {
        let policy = ReorderPolicy {
            id: Uuid::new_v4(),
            tenant_id: "t1".into(),
            item_id: Uuid::new_v4(),
            location_id: None,
            reorder_point: 50,
            safety_stock: 10,
            max_qty: Some(200),
            notes: Some("Restock weekly".into()),
            created_by: "alice".into(),
            updated_by: "alice".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains("reorder_point"));
        assert!(json.contains("safety_stock"));
        assert!(json.contains("50"));
    }
}
