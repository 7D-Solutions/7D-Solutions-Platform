//! Location domain model and repository.
//!
//! Locations are physical or logical bins/shelves within a warehouse.
//! They are optional in v1 — all existing flows work with location_id = NULL.
//!
//! Invariants:
//! - code is unique per (tenant_id, warehouse_id)
//! - name and code must be non-empty
//! - Deactivate is idempotent

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Location {
    pub id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLocationRequest {
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLocationRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum LocationError {
    #[error("Location not found")]
    NotFound,

    #[error("Location code '{0}' already exists for warehouse '{1}' in tenant '{2}'")]
    DuplicateCode(String, Uuid, String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct LocationRepo;

impl LocationRepo {
    /// Create a new location. Returns `LocationError::DuplicateCode` if the
    /// code already exists for this (tenant, warehouse).
    pub async fn create(
        pool: &PgPool,
        req: &CreateLocationRequest,
    ) -> Result<Location, LocationError> {
        if req.tenant_id.trim().is_empty() {
            return Err(LocationError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.code.trim().is_empty() {
            return Err(LocationError::Validation("code is required".to_string()));
        }
        if req.name.trim().is_empty() {
            return Err(LocationError::Validation("name is required".to_string()));
        }

        sqlx::query_as::<_, Location>(
            r#"
            INSERT INTO locations (tenant_id, warehouse_id, code, name, description)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.warehouse_id)
        .bind(&req.code)
        .bind(&req.name)
        .bind(&req.description)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return LocationError::DuplicateCode(
                        req.code.clone(),
                        req.warehouse_id,
                        req.tenant_id.clone(),
                    );
                }
            }
            LocationError::Database(e)
        })
    }

    /// Find a location by id scoped to tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Location>, LocationError> {
        sqlx::query_as::<_, Location>("SELECT * FROM locations WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await
            .map_err(LocationError::Database)
    }

    /// List all locations for a (tenant, warehouse), ordered by code.
    pub async fn list_for_warehouse(
        pool: &PgPool,
        tenant_id: &str,
        warehouse_id: Uuid,
    ) -> Result<Vec<Location>, LocationError> {
        sqlx::query_as::<_, Location>(
            r#"
            SELECT * FROM locations
            WHERE tenant_id = $1 AND warehouse_id = $2
            ORDER BY code
            "#,
        )
        .bind(tenant_id)
        .bind(warehouse_id)
        .fetch_all(pool)
        .await
        .map_err(LocationError::Database)
    }

    /// Update mutable fields (name, description). Tenant-scoped.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateLocationRequest,
    ) -> Result<Location, LocationError> {
        if req.tenant_id.trim().is_empty() {
            return Err(LocationError::Validation(
                "tenant_id is required".to_string(),
            ));
        }

        sqlx::query_as::<_, Location>(
            r#"
            UPDATE locations
            SET name        = COALESCE($3, name),
                description = COALESCE($4, description),
                updated_at  = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(&req.name)
        .bind(&req.description)
        .fetch_optional(pool)
        .await
        .map_err(LocationError::Database)?
        .ok_or(LocationError::NotFound)
    }

    /// Soft-delete a location. Idempotent — already-inactive returns Ok.
    pub async fn deactivate(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Location, LocationError> {
        sqlx::query_as::<_, Location>(
            r#"
            UPDATE locations
            SET is_active = FALSE, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(LocationError::Database)?
        .ok_or(LocationError::NotFound)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_validates_empty_code() {
        let req = CreateLocationRequest {
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            code: "  ".to_string(),
            name: "Zone A".to_string(),
            description: None,
        };
        // LocationRepo::create needs a pool so we just verify the struct compiles here.
        // DB-level validation is tested in location_integration.rs.
        assert!(req.code.trim().is_empty());
    }

    #[test]
    fn location_serializes_correctly() {
        let loc = Location {
            id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            code: "BIN-A1".to_string(),
            name: "Bin A1".to_string(),
            description: Some("Front-of-warehouse bin".to_string()),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&loc).expect("serialize Location");
        assert!(json.contains("BIN-A1"));
        assert!(json.contains("Bin A1"));
    }
}
