use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Posting category
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostingCategory {
    Labor,
    Material,
    OutsideProcessing,
    Scrap,
    Overhead,
    Other,
}

impl PostingCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Labor => "labor",
            Self::Material => "material",
            Self::OutsideProcessing => "outside_processing",
            Self::Scrap => "scrap",
            Self::Overhead => "overhead",
            Self::Other => "other",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "labor" => Some(Self::Labor),
            "material" => Some(Self::Material),
            "outside_processing" => Some(Self::OutsideProcessing),
            "scrap" => Some(Self::Scrap),
            "overhead" => Some(Self::Overhead),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

// ============================================================================
// Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CostPosting {
    pub posting_id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub posting_category: String,
    pub amount_cents: i64,
    pub quantity: Option<f64>,
    pub source_event_id: Option<Uuid>,
    pub posted_at: DateTime<Utc>,
    pub posted_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CostSummary {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub total_cost_cents: i64,
    pub labor_cost_cents: i64,
    pub material_cost_cents: i64,
    pub osp_cost_cents: i64,
    pub scrap_cost_cents: i64,
    pub overhead_cost_cents: i64,
    pub other_cost_cents: i64,
    pub posting_count: i32,
    pub last_updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PostCostRequest {
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub posting_category: PostingCategory,
    pub amount_cents: i64,
    pub quantity: Option<f64>,
    pub source_event_id: Option<Uuid>,
    pub posted_by: String,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum CostTrackingError {
    #[error("work order not found")]
    WorkOrderNotFound,
    #[error("duplicate source event — posting already exists")]
    DuplicateSourceEvent,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct CostRepo;

impl CostRepo {
    /// Post a cost line and update the summary atomically.
    ///
    /// INVARIANT: both the posting INSERT and the summary UPSERT happen in the
    /// same transaction.  If either fails, neither is committed.  This preserves
    /// the guarantee that cost_summary.total = SUM(postings) at all times.
    pub async fn post_cost(
        pool: &PgPool,
        req: &PostCostRequest,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<CostPosting, CostTrackingError> {
        let mut tx = pool.begin().await?;

        let posting = sqlx::query_as::<_, CostPosting>(
            r#"
            INSERT INTO work_order_cost_postings
                (tenant_id, work_order_id, operation_id, posting_category,
                 amount_cents, quantity, source_event_id, posted_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(req.work_order_id)
        .bind(req.operation_id)
        .bind(req.posting_category.as_str())
        .bind(req.amount_cents)
        .bind(req.quantity)
        .bind(req.source_event_id)
        .bind(&req.posted_by)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err.constraint() == Some("idx_cost_postings_source_event_idempotency") {
                    return CostTrackingError::DuplicateSourceEvent;
                }
            }
            CostTrackingError::Database(e)
        })?;

        // Update or create the summary row — same transaction.
        let category = req.posting_category.as_str();
        let delta = req.amount_cents;

        sqlx::query(
            r#"
            INSERT INTO work_order_cost_summaries
                (work_order_id, tenant_id, total_cost_cents,
                 labor_cost_cents, material_cost_cents, osp_cost_cents,
                 scrap_cost_cents, overhead_cost_cents, other_cost_cents,
                 posting_count, last_updated_at)
            VALUES ($1, $2, $3,
                    CASE WHEN $4 = 'labor'              THEN $3 ELSE 0 END,
                    CASE WHEN $4 = 'material'           THEN $3 ELSE 0 END,
                    CASE WHEN $4 = 'outside_processing' THEN $3 ELSE 0 END,
                    CASE WHEN $4 = 'scrap'              THEN $3 ELSE 0 END,
                    CASE WHEN $4 = 'overhead'           THEN $3 ELSE 0 END,
                    CASE WHEN $4 = 'other'              THEN $3 ELSE 0 END,
                    1, now())
            ON CONFLICT (work_order_id, tenant_id) DO UPDATE SET
                total_cost_cents    = work_order_cost_summaries.total_cost_cents    + $3,
                labor_cost_cents    = work_order_cost_summaries.labor_cost_cents
                                      + CASE WHEN $4 = 'labor'              THEN $3 ELSE 0 END,
                material_cost_cents = work_order_cost_summaries.material_cost_cents
                                      + CASE WHEN $4 = 'material'           THEN $3 ELSE 0 END,
                osp_cost_cents      = work_order_cost_summaries.osp_cost_cents
                                      + CASE WHEN $4 = 'outside_processing' THEN $3 ELSE 0 END,
                scrap_cost_cents    = work_order_cost_summaries.scrap_cost_cents
                                      + CASE WHEN $4 = 'scrap'              THEN $3 ELSE 0 END,
                overhead_cost_cents = work_order_cost_summaries.overhead_cost_cents
                                      + CASE WHEN $4 = 'overhead'           THEN $3 ELSE 0 END,
                other_cost_cents    = work_order_cost_summaries.other_cost_cents
                                      + CASE WHEN $4 = 'other'              THEN $3 ELSE 0 END,
                posting_count       = work_order_cost_summaries.posting_count       + 1,
                last_updated_at     = now()
            "#,
        )
        .bind(req.work_order_id)
        .bind(tenant_id)
        .bind(delta)
        .bind(category)
        .execute(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::CostPosted,
            "work_order_cost_posting",
            &posting.posting_id.to_string(),
            &events::build_cost_posted_envelope(
                posting.posting_id,
                req.work_order_id,
                req.operation_id,
                tenant_id.to_string(),
                req.posting_category,
                req.amount_cents,
                req.source_event_id,
                req.posted_by.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(posting)
    }

    pub async fn get_summary(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<CostSummary>, sqlx::Error> {
        sqlx::query_as::<_, CostSummary>(
            "SELECT * FROM work_order_cost_summaries WHERE work_order_id = $1 AND tenant_id = $2",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_postings(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<CostPosting>, sqlx::Error> {
        sqlx::query_as::<_, CostPosting>(
            "SELECT * FROM work_order_cost_postings \
             WHERE work_order_id = $1 AND tenant_id = $2 \
             ORDER BY posted_at ASC",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}
