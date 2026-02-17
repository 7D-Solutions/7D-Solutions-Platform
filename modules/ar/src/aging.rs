//! AR Aging Projection (bd-3cb, extended by bd-13p)
//!
//! Computes and stores AR aging buckets per (app_id, customer_id, currency).
//! Open balance = invoice amount − payments − credit notes − write-offs.
//!
//! Design:
//! - Projection-based: results stored in ar_aging_buckets, not computed at query time.
//! - Replayable: upsert overwrites previous snapshot deterministically.
//! - Atomic: compute + upsert + outbox event in a single transaction.
//! - Credit notes and write-offs are negative adjustments (reduce open balance).
//!
//! Buckets (by days overdue relative to due_at):
//!   current      — not yet due (due_at >= NOW() or due_at IS NULL)
//!   days_1_30    — 1–30 days past due
//!   days_31_60   — 31–60 days past due
//!   days_61_90   — 61–90 days past due
//!   days_over_90 — more than 90 days past due

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Output types
// ============================================================================

/// Pre-computed aging snapshot for one (app_id, customer_id, currency)
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AgingSnapshot {
    pub id: i32,
    pub app_id: String,
    pub customer_id: i32,
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub days_over_90_minor: i64,
    pub total_outstanding_minor: i64,
    pub invoice_count: i32,
    pub calculated_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

// ============================================================================
// Projection updater
// ============================================================================

/// Recompute and upsert the aging snapshot for a given (app_id, customer_id).
///
/// Algorithm:
/// 1. Find all non-paid, non-void invoices for the customer.
/// 2. For each invoice, subtract successful charges to get open balance.
/// 3. Bucket open balance by due_at vs NOW().
/// 4. Upsert into ar_aging_buckets.
/// 5. Emit ar.ar_aging_updated event into the outbox (same tx).
///
/// Returns the upserted snapshot.
pub async fn refresh_aging(
    db: &PgPool,
    app_id: &str,
    customer_id: i32,
) -> Result<AgingSnapshot, sqlx::Error> {
    let mut tx = db.begin().await?;

    let snapshot = refresh_aging_tx(&mut tx, app_id, customer_id).await?;

    tx.commit().await?;
    Ok(snapshot)
}

/// Transaction-aware version of refresh_aging.
///
/// Callers that need to combine aging refresh with other mutations
/// (e.g. after writing off an invoice) can use this to stay in the
/// same transaction.
pub async fn refresh_aging_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    customer_id: i32,
) -> Result<AgingSnapshot, sqlx::Error> {
    // Step 1: Compute aging buckets via SQL (deterministic, uses DB clock)
    //
    // Open balance per invoice = amount_cents − payments − credit notes − write-offs.
    // Credit notes and write-offs are negative adjustments that reduce outstanding balance.
    // An invoice with zero or negative open balance is excluded from aging.
    let computed: Option<ComputedAging> = sqlx::query_as::<_, ComputedAging>(
        r#"
        WITH open_invoices AS (
            -- All invoices that are still receivable (not paid, not void)
            SELECT
                i.id,
                i.amount_cents,
                i.currency,
                i.due_at,
                COALESCE(
                    (SELECT SUM(c.amount_cents)
                     FROM ar_charges c
                     WHERE c.invoice_id = i.id
                       AND c.status = 'succeeded'),
                    0
                ) AS paid_cents,
                COALESCE(
                    (SELECT SUM(cn.amount_minor)
                     FROM ar_credit_notes cn
                     WHERE cn.invoice_id = i.id
                       AND cn.status = 'issued'),
                    0
                ) AS credit_note_cents,
                COALESCE(
                    (SELECT SUM(wo.written_off_amount_minor)
                     FROM ar_invoice_write_offs wo
                     WHERE wo.invoice_id = i.id
                       AND wo.status = 'written_off'),
                    0
                ) AS written_off_cents
            FROM ar_invoices i
            WHERE i.app_id = $1
              AND i.ar_customer_id = $2
              AND i.status NOT IN ('paid', 'void', 'draft')
        ),
        open_balances AS (
            SELECT
                currency,
                GREATEST(0, amount_cents - paid_cents - credit_note_cents - written_off_cents) AS open_balance,
                due_at,
                CASE
                    WHEN due_at IS NULL OR due_at >= NOW() THEN 'current'
                    WHEN due_at >= NOW() - INTERVAL '30 days' THEN 'days_1_30'
                    WHEN due_at >= NOW() - INTERVAL '60 days' THEN 'days_31_60'
                    WHEN due_at >= NOW() - INTERVAL '90 days' THEN 'days_61_90'
                    ELSE 'days_over_90'
                END AS bucket
            FROM open_invoices
            WHERE GREATEST(0, amount_cents - paid_cents - credit_note_cents - written_off_cents) > 0
        )
        SELECT
            COALESCE(MAX(currency), 'usd') AS currency,
            COALESCE(SUM(CASE WHEN bucket = 'current'    THEN open_balance ELSE 0 END), 0)::BIGINT AS current_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_1_30'  THEN open_balance ELSE 0 END), 0)::BIGINT AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_31_60' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_61_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_over_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_over_90_minor,
            COALESCE(SUM(open_balance), 0)::BIGINT AS total_outstanding_minor,
            COUNT(*)::BIGINT AS invoice_count
        FROM open_balances
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .fetch_optional(&mut **tx)
    .await?;

    let aging = computed.unwrap_or_default();

    // Step 2: Upsert into projection table
    let snapshot = sqlx::query_as::<_, AgingSnapshot>(
        r#"
        INSERT INTO ar_aging_buckets (
            app_id, customer_id, currency,
            current_minor, days_1_30_minor, days_31_60_minor,
            days_61_90_minor, days_over_90_minor, total_outstanding_minor,
            invoice_count, calculated_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW(), NOW())
        ON CONFLICT (app_id, customer_id, currency)
        DO UPDATE SET
            current_minor           = EXCLUDED.current_minor,
            days_1_30_minor         = EXCLUDED.days_1_30_minor,
            days_31_60_minor        = EXCLUDED.days_31_60_minor,
            days_61_90_minor        = EXCLUDED.days_61_90_minor,
            days_over_90_minor      = EXCLUDED.days_over_90_minor,
            total_outstanding_minor = EXCLUDED.total_outstanding_minor,
            invoice_count           = EXCLUDED.invoice_count,
            calculated_at           = NOW(),
            updated_at              = NOW()
        RETURNING id, app_id, customer_id, currency,
                  current_minor, days_1_30_minor, days_31_60_minor,
                  days_61_90_minor, days_over_90_minor, total_outstanding_minor,
                  invoice_count, calculated_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(&aging.currency)
    .bind(aging.current_minor)
    .bind(aging.days_1_30_minor)
    .bind(aging.days_31_60_minor)
    .bind(aging.days_61_90_minor)
    .bind(aging.days_over_90_minor)
    .bind(aging.total_outstanding_minor)
    .bind(aging.invoice_count)
    .fetch_one(&mut **tx)
    .await?;

    // Step 3: Emit ar.ar_aging_updated event into outbox (same tx)
    use crate::events::contracts::{
        build_ar_aging_updated_envelope, AgingBuckets, ArAgingUpdatedPayload,
        EVENT_TYPE_AR_AGING_UPDATED,
    };
    use crate::events::outbox::enqueue_event_tx;

    let event_payload = ArAgingUpdatedPayload {
        tenant_id: app_id.to_string(),
        invoice_count: snapshot.invoice_count as i64,
        buckets: AgingBuckets {
            current_minor: snapshot.current_minor,
            days_1_30_minor: snapshot.days_1_30_minor,
            days_31_60_minor: snapshot.days_31_60_minor,
            days_61_90_minor: snapshot.days_61_90_minor,
            days_over_90_minor: snapshot.days_over_90_minor,
            total_outstanding_minor: snapshot.total_outstanding_minor,
            currency: snapshot.currency.clone(),
        },
        calculated_at: chrono::Utc::now(),
    };

    let event_id = Uuid::new_v4();
    let envelope = build_ar_aging_updated_envelope(
        event_id,
        app_id.to_string(),
        event_id.to_string(),
        None,
        event_payload,
    );

    enqueue_event_tx(
        tx,
        EVENT_TYPE_AR_AGING_UPDATED,
        "aging",
        &snapshot.customer_id.to_string(),
        &envelope,
    )
    .await?;

    tracing::info!(
        app_id = %app_id,
        customer_id = %customer_id,
        total_outstanding = %snapshot.total_outstanding_minor,
        "Aging projection refreshed"
    );

    Ok(snapshot)
}

// ============================================================================
// Read queries
// ============================================================================

/// Fetch aging snapshot for all customers of an app
pub async fn get_aging_for_app(
    db: &PgPool,
    app_id: &str,
) -> Result<Vec<AgingSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, AgingSnapshot>(
        r#"
        SELECT id, app_id, customer_id, currency,
               current_minor, days_1_30_minor, days_31_60_minor,
               days_61_90_minor, days_over_90_minor, total_outstanding_minor,
               invoice_count, calculated_at, updated_at
        FROM ar_aging_buckets
        WHERE app_id = $1
        ORDER BY total_outstanding_minor DESC
        "#,
    )
    .bind(app_id)
    .fetch_all(db)
    .await
}

/// Fetch aging snapshot for a specific customer
pub async fn get_aging_for_customer(
    db: &PgPool,
    app_id: &str,
    customer_id: i32,
) -> Result<Option<AgingSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, AgingSnapshot>(
        r#"
        SELECT id, app_id, customer_id, currency,
               current_minor, days_1_30_minor, days_31_60_minor,
               days_61_90_minor, days_over_90_minor, total_outstanding_minor,
               invoice_count, calculated_at, updated_at
        FROM ar_aging_buckets
        WHERE app_id = $1 AND customer_id = $2
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .fetch_optional(db)
    .await
}

// ============================================================================
// Internal helper: intermediate row from the SQL computation
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
struct ComputedAging {
    pub currency: String,
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub days_over_90_minor: i64,
    pub total_outstanding_minor: i64,
    pub invoice_count: i64,
}

impl Default for ComputedAging {
    fn default() -> Self {
        Self {
            currency: "usd".to_string(),
            current_minor: 0,
            days_1_30_minor: 0,
            days_31_60_minor: 0,
            days_61_90_minor: 0,
            days_over_90_minor: 0,
            total_outstanding_minor: 0,
            invoice_count: 0,
        }
    }
}
