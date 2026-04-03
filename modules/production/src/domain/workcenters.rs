use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::idempotency::{check_idempotency, store_idempotency_key, IdempotencyError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Workcenter {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub capacity: Option<i32>,
    pub cost_rate_minor: Option<i64>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateWorkcenterRequest {
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub capacity: Option<i32>,
    pub cost_rate_minor: Option<i64>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWorkcenterRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub capacity: Option<i32>,
    pub cost_rate_minor: Option<i64>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum WorkcenterError {
    #[error("Workcenter not found")]
    NotFound,

    #[error("Workcenter code '{0}' already exists for tenant '{1}'")]
    DuplicateCode(String, String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflicting idempotency key")]
    ConflictingIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct WorkcenterRepo;

impl WorkcenterRepo {
    pub async fn create(
        pool: &PgPool,
        req: &CreateWorkcenterRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<Workcenter, WorkcenterError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WorkcenterError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.code.trim().is_empty() {
            return Err(WorkcenterError::Validation("code is required".to_string()));
        }
        if req.name.trim().is_empty() {
            return Err(WorkcenterError::Validation("name is required".to_string()));
        }

        let request_hash = serde_json::to_string(req)
            .map_err(|e| WorkcenterError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, &req.tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let wc: Workcenter = serde_json::from_str(&cached).map_err(|e| {
                        WorkcenterError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(wc);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(WorkcenterError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(WorkcenterError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(WorkcenterError::Database(sqlx::Error::Protocol(
                        e.to_string(),
                    )));
                }
            }
        }

        let wc = sqlx::query_as::<_, Workcenter>(
            r#"
            INSERT INTO workcenters (tenant_id, code, name, description, capacity, cost_rate_minor)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(&req.code)
        .bind(&req.name)
        .bind(&req.description)
        .bind(req.capacity)
        .bind(req.cost_rate_minor)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return WorkcenterError::DuplicateCode(
                        req.code.clone(),
                        req.tenant_id.clone(),
                    );
                }
            }
            WorkcenterError::Database(e)
        })?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::WorkcenterCreated,
            "workcenter",
            &wc.workcenter_id.to_string(),
            &events::build_workcenter_created_envelope(
                wc.workcenter_id,
                req.tenant_id.clone(),
                req.code.clone(),
                req.name.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&wc)
                .map_err(|e| WorkcenterError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                &req.tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(wc)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Workcenter>, WorkcenterError> {
        sqlx::query_as::<_, Workcenter>(
            "SELECT * FROM workcenters WHERE workcenter_id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(WorkcenterError::Database)
    }

    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        page: i64,
        page_size: i64,
        name: Option<&str>,
    ) -> Result<(Vec<Workcenter>, i64), WorkcenterError> {
        let limit = page_size.clamp(1, 200);
        let offset = (page.max(1) - 1) * limit;

        let items = sqlx::query_as::<_, Workcenter>(
            "SELECT * FROM workcenters WHERE tenant_id = $1 AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%') ORDER BY code LIMIT $3 OFFSET $4",
        )
        .bind(tenant_id)
        .bind(name)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(WorkcenterError::Database)?;

        let (total,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workcenters WHERE tenant_id = $1 AND ($2::text IS NULL OR name ILIKE '%' || $2 || '%')",
        )
        .bind(tenant_id)
        .bind(name)
        .fetch_one(pool)
        .await
        .map_err(WorkcenterError::Database)?;

        Ok((items, total))
    }

    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateWorkcenterRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<Workcenter, WorkcenterError> {
        if req.tenant_id.trim().is_empty() {
            return Err(WorkcenterError::Validation(
                "tenant_id is required".to_string(),
            ));
        }

        let mut tx = pool.begin().await?;

        let wc = sqlx::query_as::<_, Workcenter>(
            r#"
            UPDATE workcenters
            SET name            = COALESCE($3, name),
                description     = COALESCE($4, description),
                capacity        = COALESCE($5, capacity),
                cost_rate_minor = COALESCE($6, cost_rate_minor),
                updated_at      = NOW()
            WHERE workcenter_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(req.capacity)
        .bind(req.cost_rate_minor)
        .fetch_optional(&mut *tx)
        .await
        .map_err(WorkcenterError::Database)?
        .ok_or(WorkcenterError::NotFound)?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::WorkcenterUpdated,
            "workcenter",
            &wc.workcenter_id.to_string(),
            &events::build_workcenter_updated_envelope(
                wc.workcenter_id,
                req.tenant_id.clone(),
                wc.code.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(wc)
    }

    pub async fn deactivate(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<Workcenter, WorkcenterError> {
        let mut tx = pool.begin().await?;

        let wc = sqlx::query_as::<_, Workcenter>(
            r#"
            UPDATE workcenters
            SET is_active = FALSE, updated_at = NOW()
            WHERE workcenter_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(WorkcenterError::Database)?
        .ok_or(WorkcenterError::NotFound)?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::WorkcenterDeactivated,
            "workcenter",
            &wc.workcenter_id.to_string(),
            &events::build_workcenter_deactivated_envelope(
                wc.workcenter_id,
                tenant_id.to_string(),
                wc.code.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(wc)
    }
}
