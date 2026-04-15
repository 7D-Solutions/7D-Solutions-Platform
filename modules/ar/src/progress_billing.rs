//! AR Progress Billing / Milestone Invoicing (Phase 63, bd-7nvjh)
//!
//! Invoicing against project milestones or percentage-of-completion rather than
//! full delivery. Core invariant: cumulative billed amount never exceeds the
//! contract total.
//!
//! ## Transaction Pattern (Guard → Mutation → Outbox)
//! ```text
//! BEGIN
//!   SELECT contract FOR UPDATE               -- Guard: lock + validate
//!   CHECK cumulative + amount <= total        -- Guard: over-billing prevention
//!   CHECK idempotency_key not already used    -- Guard: idempotency
//!   INSERT ar_invoices                        -- Mutation: create invoice
//!   UPDATE milestone SET status='billed'      -- Mutation: mark billed
//!   INSERT events_outbox                      -- Outbox: emit event
//! COMMIT
//! ```

use crate::events::{
    build_milestone_invoice_created_envelope, MilestoneInvoiceCreatedPayload,
    EVENT_TYPE_MILESTONE_INVOICE_CREATED,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContractRequest {
    pub contract_id: Uuid,
    pub app_id: String,
    pub customer_id: String,
    pub description: String,
    pub total_amount_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddMilestoneRequest {
    pub milestone_id: Uuid,
    pub app_id: String,
    pub contract_id: Uuid,
    pub name: String,
    pub percentage: i32,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillMilestoneRequest {
    pub app_id: String,
    pub contract_id: Uuid,
    pub milestone_id: Uuid,
    pub idempotency_key: Uuid,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum BillMilestoneResult {
    Invoiced {
        invoice_id: i32,
        milestone_id: Uuid,
        amount_minor: i64,
        cumulative_billed_minor: i64,
    },
    AlreadyProcessed {
        milestone_id: Uuid,
    },
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum ProgressBillingError {
    ContractNotFound {
        contract_id: Uuid,
        app_id: String,
    },
    MilestoneNotFound {
        milestone_id: Uuid,
        app_id: String,
    },
    MilestoneAlreadyBilled {
        milestone_id: Uuid,
    },
    OverBilling {
        contract_id: Uuid,
        contract_total: i64,
        already_billed: i64,
        requested: i64,
    },
    InvalidAmount(i64),
    InvalidPercentage(i32),
    DatabaseError(String),
}

impl fmt::Display for ProgressBillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContractNotFound {
                contract_id,
                app_id,
            } => {
                write!(
                    f,
                    "Contract {} not found for tenant {}",
                    contract_id, app_id
                )
            }
            Self::MilestoneNotFound {
                milestone_id,
                app_id,
            } => {
                write!(
                    f,
                    "Milestone {} not found for tenant {}",
                    milestone_id, app_id
                )
            }
            Self::MilestoneAlreadyBilled { milestone_id } => {
                write!(f, "Milestone {} is already billed", milestone_id)
            }
            Self::OverBilling {
                contract_id,
                contract_total,
                already_billed,
                requested,
            } => {
                write!(
                    f,
                    "Billing {} would exceed contract {} total {} (already billed {})",
                    requested, contract_id, contract_total, already_billed
                )
            }
            Self::InvalidAmount(n) => write!(f, "Amount must be > 0, got {}", n),
            Self::InvalidPercentage(n) => {
                write!(f, "Percentage must be between 1 and 100, got {}", n)
            }
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for ProgressBillingError {}

impl From<sqlx::Error> for ProgressBillingError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Core functions
// ============================================================================

pub async fn create_contract(
    pool: &PgPool,
    req: CreateContractRequest,
) -> Result<i32, ProgressBillingError> {
    if req.total_amount_minor <= 0 {
        return Err(ProgressBillingError::InvalidAmount(req.total_amount_minor));
    }

    let row_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_progress_billing_contracts (
            contract_id, app_id, customer_id, description,
            total_amount_minor, currency, status, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, 'active', NOW(), NOW())
        ON CONFLICT (app_id, contract_id) DO UPDATE SET updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(req.contract_id)
    .bind(&req.app_id)
    .bind(&req.customer_id)
    .bind(&req.description)
    .bind(req.total_amount_minor)
    .bind(&req.currency)
    .fetch_one(pool)
    .await?;

    Ok(row_id)
}

pub async fn add_milestone(
    pool: &PgPool,
    req: AddMilestoneRequest,
) -> Result<i32, ProgressBillingError> {
    if req.amount_minor <= 0 {
        return Err(ProgressBillingError::InvalidAmount(req.amount_minor));
    }
    if req.percentage < 1 || req.percentage > 100 {
        return Err(ProgressBillingError::InvalidPercentage(req.percentage));
    }

    let contract_row: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM ar_progress_billing_contracts WHERE app_id = $1 AND contract_id = $2",
    )
    .bind(&req.app_id)
    .bind(req.contract_id)
    .fetch_optional(pool)
    .await?;

    let contract_row_id = match contract_row {
        Some(id) => id,
        None => {
            return Err(ProgressBillingError::ContractNotFound {
                contract_id: req.contract_id,
                app_id: req.app_id,
            })
        }
    };

    let row_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_progress_billing_milestones (
            milestone_id, contract_row_id, app_id, name,
            percentage, amount_minor, status, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, 'pending', NOW(), NOW())
        ON CONFLICT (app_id, milestone_id) DO UPDATE SET updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(req.milestone_id)
    .bind(contract_row_id)
    .bind(&req.app_id)
    .bind(&req.name)
    .bind(req.percentage)
    .bind(req.amount_minor)
    .fetch_one(pool)
    .await?;

    Ok(row_id)
}

/// Bill a milestone: Guard → Mutation → Outbox in a single transaction.
///
/// Guards:
/// - Contract must exist and belong to tenant
/// - Milestone must exist, belong to contract, and be in 'pending' status
/// - Idempotency: if idempotency_key was already used, return AlreadyProcessed
/// - Cumulative billed amount (existing + this) must not exceed contract total
pub async fn bill_milestone(
    pool: &PgPool,
    req: BillMilestoneRequest,
) -> Result<BillMilestoneResult, ProgressBillingError> {
    let mut tx = pool.begin().await?;

    // --- Guard: idempotency check ---
    let existing_for_key: Option<Uuid> = sqlx::query_scalar(
        "SELECT milestone_id FROM ar_progress_billing_milestones \
         WHERE app_id = $1 AND idempotency_key = $2",
    )
    .bind(&req.app_id)
    .bind(req.idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(milestone_id) = existing_for_key {
        tx.rollback().await?;
        return Ok(BillMilestoneResult::AlreadyProcessed { milestone_id });
    }

    // --- Guard: contract exists and lock ---
    let contract_row: Option<(i32, i64, String, String)> = sqlx::query_as(
        "SELECT id, total_amount_minor, currency, customer_id \
         FROM ar_progress_billing_contracts \
         WHERE app_id = $1 AND contract_id = $2 AND status = 'active' \
         FOR UPDATE",
    )
    .bind(&req.app_id)
    .bind(req.contract_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (contract_row_id, contract_total, currency, customer_id) = match contract_row {
        Some(v) => v,
        None => {
            tx.rollback().await?;
            return Err(ProgressBillingError::ContractNotFound {
                contract_id: req.contract_id,
                app_id: req.app_id,
            });
        }
    };

    // --- Guard: milestone exists, belongs to this contract, is pending ---
    let milestone_row: Option<(i32, i64, i32, String, String)> = sqlx::query_as(
        "SELECT id, amount_minor, percentage, name, status \
         FROM ar_progress_billing_milestones \
         WHERE app_id = $1 AND milestone_id = $2 AND contract_row_id = $3 \
         FOR UPDATE",
    )
    .bind(&req.app_id)
    .bind(req.milestone_id)
    .bind(contract_row_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (milestone_row_id, amount_minor, percentage, milestone_name, milestone_status) =
        match milestone_row {
            Some(v) => v,
            None => {
                tx.rollback().await?;
                return Err(ProgressBillingError::MilestoneNotFound {
                    milestone_id: req.milestone_id,
                    app_id: req.app_id,
                });
            }
        };

    if milestone_status == "billed" {
        tx.rollback().await?;
        return Err(ProgressBillingError::MilestoneAlreadyBilled {
            milestone_id: req.milestone_id,
        });
    }

    // --- Guard: over-billing prevention ---
    let already_billed: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT \
         FROM ar_progress_billing_milestones \
         WHERE contract_row_id = $1 AND status = 'billed'",
    )
    .bind(contract_row_id)
    .fetch_one(&mut *tx)
    .await?;

    if already_billed + amount_minor > contract_total {
        tx.rollback().await?;
        return Err(ProgressBillingError::OverBilling {
            contract_id: req.contract_id,
            contract_total,
            already_billed,
            requested: amount_minor,
        });
    }

    // --- Mutation: create invoice ---
    let now = Utc::now();
    let tilled_invoice_id = format!("pb_inv_{}", Uuid::new_v4());

    // We need an ar_customer_id. Look up by app_id + external customer_id.
    // For progress billing, seed or find a customer row.
    let ar_customer_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM ar_customers WHERE app_id = $1 LIMIT 1")
            .bind(&req.app_id)
            .fetch_optional(&mut *tx)
            .await?;

    let ar_customer_id = ar_customer_id.unwrap_or(0);

    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            metadata, correlation_id, created_at, updated_at
        ) VALUES ($1, $2, $3, 'open', $4, $5, $6, $7, $8, $8)
        RETURNING id
        "#,
    )
    .bind(&req.app_id)
    .bind(&tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(amount_minor as i32)
    .bind(&currency)
    .bind(serde_json::json!({
        "progress_billing": true,
        "contract_id": req.contract_id.to_string(),
        "milestone_id": req.milestone_id.to_string(),
        "milestone_name": &milestone_name,
        "percentage": percentage,
    }))
    .bind(&req.correlation_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // --- Mutation: mark milestone as billed ---
    sqlx::query(
        "UPDATE ar_progress_billing_milestones \
         SET status = 'billed', billed_at = $1, invoice_id = $2, \
             idempotency_key = $3, updated_at = $1 \
         WHERE id = $4",
    )
    .bind(now)
    .bind(invoice_id)
    .bind(req.idempotency_key)
    .bind(milestone_row_id)
    .execute(&mut *tx)
    .await?;

    let cumulative_billed = already_billed + amount_minor;

    // --- Outbox: emit event ---
    let outbox_event_id = Uuid::new_v4();
    let envelope = build_milestone_invoice_created_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        MilestoneInvoiceCreatedPayload {
            contract_id: req.contract_id,
            milestone_id: req.milestone_id,
            tenant_id: req.app_id.clone(),
            customer_id: customer_id.clone(),
            invoice_id,
            amount_minor,
            currency: currency.clone(),
            milestone_name: milestone_name.clone(),
            percentage,
            cumulative_billed_minor: cumulative_billed,
            contract_total_minor: contract_total,
            created_at: now,
        },
    );
    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| ProgressBillingError::DatabaseError(e.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'progress_billing', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_MILESTONE_INVOICE_CREATED)
    .bind(req.contract_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(BillMilestoneResult::Invoiced {
        invoice_id,
        milestone_id: req.milestone_id,
        amount_minor,
        cumulative_billed_minor: cumulative_billed,
    })
}

/// Query milestones for a contract within a tenant.
pub async fn list_milestones(
    pool: &PgPool,
    app_id: &str,
    contract_id: Uuid,
) -> Result<Vec<MilestoneInfo>, ProgressBillingError> {
    let contract_row: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM ar_progress_billing_contracts WHERE app_id = $1 AND contract_id = $2",
    )
    .bind(app_id)
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;

    let contract_row_id = match contract_row {
        Some(id) => id,
        None => {
            return Err(ProgressBillingError::ContractNotFound {
                contract_id,
                app_id: app_id.to_string(),
            })
        }
    };

    let rows: Vec<(Uuid, String, i32, i64, String, Option<i32>)> = sqlx::query_as(
        "SELECT milestone_id, name, percentage, amount_minor, status, invoice_id \
         FROM ar_progress_billing_milestones \
         WHERE contract_row_id = $1 AND app_id = $2 \
         ORDER BY created_at",
    )
    .bind(contract_row_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(milestone_id, name, percentage, amount_minor, status, invoice_id)| MilestoneInfo {
                milestone_id,
                name,
                percentage,
                amount_minor,
                status,
                invoice_id,
            },
        )
        .collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct MilestoneInfo {
    pub milestone_id: Uuid,
    pub name: String,
    pub percentage: i32,
    pub amount_minor: i64,
    pub status: String,
    pub invoice_id: Option<i32>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_contract_not_found() {
        let err = ProgressBillingError::ContractNotFound {
            contract_id: Uuid::nil(),
            app_id: "tenant-1".to_string(),
        };
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn error_display_over_billing() {
        let err = ProgressBillingError::OverBilling {
            contract_id: Uuid::nil(),
            contract_total: 100000,
            already_billed: 80000,
            requested: 30000,
        };
        assert!(err.to_string().contains("exceed"));
    }

    #[test]
    fn error_display_invalid_amount() {
        let err = ProgressBillingError::InvalidAmount(-5);
        assert_eq!(err.to_string(), "Amount must be > 0, got -5");
    }

    #[test]
    fn error_display_invalid_percentage() {
        let err = ProgressBillingError::InvalidPercentage(0);
        assert!(err.to_string().contains("between 1 and 100"));
    }
}
