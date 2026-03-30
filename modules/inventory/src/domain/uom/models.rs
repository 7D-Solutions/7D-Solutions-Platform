//! Unit-of-measure domain models and repository.
//!
//! Tenant-scoped UoM catalog + item-level conversion factors.
//!
//! Invariants enforced here (application layer):
//!   - UoM code and name must be non-empty
//!   - Conversion factor must be positive
//!   - from_uom_id must differ from to_uom_id
//!
//! Invariants enforced by DB constraints:
//!   - UoM code is unique per tenant
//!   - (item_id, from_uom_id, to_uom_id) is unique
//!   - factor > 0 (CHECK constraint)
//!   - from_uom_id != to_uom_id (CHECK constraint)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Domain models
// ============================================================================

/// A unit of measure in the tenant's catalog (e.g. "ea", "kg", "box").
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Uom {
    pub id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// A directional conversion factor between two UoMs for a specific item.
///
/// Semantics: `quantity_in_from_uom * factor = quantity_in_to_uom`
/// Example: 1 box = 12 ea → from=box, to=ea, factor=12.0
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ItemUomConversion {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub from_uom_id: Uuid,
    pub to_uom_id: Uuid,
    pub factor: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

/// Input for POST /api/inventory/uoms
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUomRequest {
    pub tenant_id: String,
    pub code: String,
    pub name: String,
}

/// Input for POST /api/inventory/items/:id/uom-conversions
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConversionRequest {
    pub tenant_id: String,
    pub from_uom_id: Uuid,
    pub to_uom_id: Uuid,
    /// Multiply from_uom quantity by this factor to get to_uom quantity. Must be > 0.
    pub factor: f64,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum UomError {
    #[error("UoM code '{0}' already exists for tenant '{1}'")]
    DuplicateCode(String, String),

    #[error("Conversion already exists for this item and direction")]
    DuplicateConversion,

    #[error("UoM not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation
// ============================================================================

impl CreateUomRequest {
    pub fn validate(&self) -> Result<(), UomError> {
        if self.tenant_id.trim().is_empty() {
            return Err(UomError::Validation("tenant_id must not be empty".into()));
        }
        if self.code.trim().is_empty() {
            return Err(UomError::Validation("code must not be empty".into()));
        }
        if self.name.trim().is_empty() {
            return Err(UomError::Validation("name must not be empty".into()));
        }
        Ok(())
    }
}

impl CreateConversionRequest {
    pub fn validate(&self) -> Result<(), UomError> {
        if self.tenant_id.trim().is_empty() {
            return Err(UomError::Validation("tenant_id must not be empty".into()));
        }
        if self.factor <= 0.0 {
            return Err(UomError::Validation("factor must be positive".into()));
        }
        if self.from_uom_id == self.to_uom_id {
            return Err(UomError::Validation(
                "from_uom_id and to_uom_id must differ".into(),
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Repositories
// ============================================================================

pub struct UomRepo;

impl UomRepo {
    /// Create a new UoM in the tenant's catalog.
    ///
    /// Returns DuplicateCode if the (tenant_id, code) pair already exists.
    pub async fn create(pool: &PgPool, req: &CreateUomRequest) -> Result<Uom, UomError> {
        req.validate()?;

        sqlx::query_as::<_, Uom>(
            r#"
            INSERT INTO uoms (tenant_id, code, name)
            VALUES ($1, $2, $3)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.code.trim())
        .bind(req.name.trim())
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return UomError::DuplicateCode(req.code.clone(), req.tenant_id.clone());
                }
            }
            UomError::Database(e)
        })
    }

    /// List all UoMs for a tenant, ordered by code.
    pub async fn list_for_tenant(pool: &PgPool, tenant_id: &str) -> Result<Vec<Uom>, UomError> {
        sqlx::query_as::<_, Uom>("SELECT * FROM uoms WHERE tenant_id = $1 ORDER BY code")
            .bind(tenant_id)
            .fetch_all(pool)
            .await
            .map_err(UomError::Database)
    }
}

pub struct ConversionRepo;

impl ConversionRepo {
    /// Add a directional UoM conversion for an item.
    ///
    /// Returns DuplicateConversion if the (item_id, from_uom_id, to_uom_id) tuple
    /// already exists.
    pub async fn create(
        pool: &PgPool,
        item_id: Uuid,
        req: &CreateConversionRequest,
    ) -> Result<ItemUomConversion, UomError> {
        req.validate()?;

        sqlx::query_as::<_, ItemUomConversion>(
            r#"
            INSERT INTO item_uom_conversions
                (tenant_id, item_id, from_uom_id, to_uom_id, factor)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(item_id)
        .bind(req.from_uom_id)
        .bind(req.to_uom_id)
        .bind(req.factor)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return UomError::DuplicateConversion;
                }
            }
            UomError::Database(e)
        })
    }

    /// List all conversions for an item, scoped to tenant.
    pub async fn list_for_item(
        pool: &PgPool,
        item_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<ItemUomConversion>, UomError> {
        sqlx::query_as::<_, ItemUomConversion>(
            r#"
            SELECT * FROM item_uom_conversions
            WHERE item_id = $1 AND tenant_id = $2
            ORDER BY created_at
            "#,
        )
        .bind(item_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(UomError::Database)
    }
}

// ============================================================================
// Unit tests (pure validation; DB tests live in integration suite)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_uom_empty_code_rejected() {
        let req = CreateUomRequest {
            tenant_id: "t1".into(),
            code: "  ".into(),
            name: "Each".into(),
        };
        assert!(matches!(req.validate(), Err(UomError::Validation(_))));
    }

    #[test]
    fn create_uom_empty_name_rejected() {
        let req = CreateUomRequest {
            tenant_id: "t1".into(),
            code: "ea".into(),
            name: "".into(),
        };
        assert!(matches!(req.validate(), Err(UomError::Validation(_))));
    }

    #[test]
    fn create_uom_valid() {
        let req = CreateUomRequest {
            tenant_id: "t1".into(),
            code: "ea".into(),
            name: "Each".into(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn conversion_zero_factor_rejected() {
        let req = CreateConversionRequest {
            tenant_id: "t1".into(),
            from_uom_id: Uuid::new_v4(),
            to_uom_id: Uuid::new_v4(),
            factor: 0.0,
        };
        assert!(matches!(req.validate(), Err(UomError::Validation(_))));
    }

    #[test]
    fn conversion_negative_factor_rejected() {
        let req = CreateConversionRequest {
            tenant_id: "t1".into(),
            from_uom_id: Uuid::new_v4(),
            to_uom_id: Uuid::new_v4(),
            factor: -1.0,
        };
        assert!(matches!(req.validate(), Err(UomError::Validation(_))));
    }

    #[test]
    fn conversion_same_uom_rejected() {
        let id = Uuid::new_v4();
        let req = CreateConversionRequest {
            tenant_id: "t1".into(),
            from_uom_id: id,
            to_uom_id: id,
            factor: 1.0,
        };
        assert!(matches!(req.validate(), Err(UomError::Validation(_))));
    }

    #[test]
    fn conversion_valid() {
        let req = CreateConversionRequest {
            tenant_id: "t1".into(),
            from_uom_id: Uuid::new_v4(),
            to_uom_id: Uuid::new_v4(),
            factor: 12.0,
        };
        assert!(req.validate().is_ok());
    }
}
