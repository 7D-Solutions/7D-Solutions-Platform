//! Payment terms service — Guard → Mutation → Outbox DB operations.
//!
//! All writes follow the pattern:
//!   1. Guard: validate preconditions (no duplicate term_code, idempotency check)
//!   2. Mutation: insert/update payment_terms row
//!   3. Outbox: enqueue event atomically in the same transaction

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::payment_terms::{
    build_payment_terms_created_envelope, PaymentTermsCreatedPayload,
    EVENT_TYPE_PAYMENT_TERMS_CREATED,
};
use crate::outbox::enqueue_event_tx;

use super::{
    compute_discount_amount, compute_discount_date, compute_due_date, AssignTermsResult,
    CreatePaymentTermsRequest, PaymentTerms, PaymentTermsError, UpdatePaymentTermsRequest,
};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single payment terms record. Returns None if not found for this tenant.
pub async fn get_terms(
    pool: &PgPool,
    tenant_id: &str,
    term_id: Uuid,
) -> Result<Option<PaymentTerms>, PaymentTermsError> {
    let row = sqlx::query_as::<_, PaymentTerms>(
        r#"
        SELECT term_id, tenant_id, term_code, description, days_due,
               discount_pct, discount_days, installment_schedule,
               idempotency_key, is_active, created_at, updated_at
        FROM payment_terms
        WHERE term_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(term_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// List active payment terms for a tenant.
pub async fn list_terms(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<PaymentTerms>, PaymentTermsError> {
    let rows = if include_inactive {
        sqlx::query_as::<_, PaymentTerms>(
            r#"
            SELECT term_id, tenant_id, term_code, description, days_due,
                   discount_pct, discount_days, installment_schedule,
                   idempotency_key, is_active, created_at, updated_at
            FROM payment_terms
            WHERE tenant_id = $1
            ORDER BY term_code ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, PaymentTerms>(
            r#"
            SELECT term_id, tenant_id, term_code, description, days_due,
                   discount_pct, discount_days, installment_schedule,
                   idempotency_key, is_active, created_at, updated_at
            FROM payment_terms
            WHERE tenant_id = $1 AND is_active = TRUE
            ORDER BY term_code ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };

    Ok(rows)
}

// ============================================================================
// Writes
// ============================================================================

/// Create payment terms with Guard → Mutation → Outbox atomicity.
///
/// - Idempotency: if idempotency_key matches an existing row, returns it.
/// - Guard: validates request, checks for duplicate term_code.
/// - Outbox: enqueues ap.payment_terms_created event.
pub async fn create_terms(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePaymentTermsRequest,
    correlation_id: String,
) -> Result<PaymentTerms, PaymentTermsError> {
    req.validate()?;

    // Guard: idempotency check — if key already used, return existing
    if let Some(ref key) = req.idempotency_key {
        let existing: Option<PaymentTerms> = sqlx::query_as(
            r#"
            SELECT term_id, tenant_id, term_code, description, days_due,
                   discount_pct, discount_days, installment_schedule,
                   idempotency_key, is_active, created_at, updated_at
            FROM payment_terms
            WHERE tenant_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = existing {
            return Ok(row);
        }
    }

    let term_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let discount_pct = req.discount_pct.unwrap_or(0.0);
    let discount_days = req.discount_days.unwrap_or(0);
    let description = req.description.as_deref().unwrap_or("");

    let mut tx = pool.begin().await?;

    // Mutation: insert payment terms
    let terms: PaymentTerms = sqlx::query_as(
        r#"
        INSERT INTO payment_terms (
            term_id, tenant_id, term_code, description, days_due,
            discount_pct, discount_days, installment_schedule,
            idempotency_key, is_active, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, TRUE, $10, $10)
        RETURNING
            term_id, tenant_id, term_code, description, days_due,
            discount_pct, discount_days, installment_schedule,
            idempotency_key, is_active, created_at, updated_at
        "#,
    )
    .bind(term_id)
    .bind(tenant_id)
    .bind(req.term_code.trim())
    .bind(description)
    .bind(req.days_due)
    .bind(discount_pct)
    .bind(discount_days)
    .bind(&req.installment_schedule)
    .bind(&req.idempotency_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db) = e {
            if db.code().as_deref() == Some("23505") {
                let msg = db.message();
                if msg.contains("idempotency") {
                    return PaymentTermsError::DuplicateIdempotencyKey(
                        req.idempotency_key.clone().unwrap_or_default(),
                    );
                }
                return PaymentTermsError::DuplicateTermCode(req.term_code.trim().to_string());
            }
        }
        PaymentTermsError::Database(e)
    })?;

    // Outbox: enqueue payment_terms_created event
    let payload = PaymentTermsCreatedPayload {
        term_id,
        tenant_id: tenant_id.to_string(),
        term_code: req.term_code.trim().to_string(),
        description: description.to_string(),
        days_due: req.days_due,
        discount_pct,
        discount_days,
        created_at: now,
    };

    let envelope = build_payment_terms_created_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PAYMENT_TERMS_CREATED,
        "payment_terms",
        &term_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(terms)
}

/// Update payment terms. Only modifiable fields are updated.
pub async fn update_terms(
    pool: &PgPool,
    tenant_id: &str,
    term_id: Uuid,
    req: &UpdatePaymentTermsRequest,
) -> Result<PaymentTerms, PaymentTermsError> {
    req.validate()?;

    // Guard: terms must exist for this tenant
    let existing = get_terms(pool, tenant_id, term_id).await?;
    if existing.is_none() {
        return Err(PaymentTermsError::NotFound(term_id));
    }

    let terms: PaymentTerms = sqlx::query_as(
        r#"
        UPDATE payment_terms
        SET description = COALESCE($3, description),
            days_due = COALESCE($4, days_due),
            discount_pct = COALESCE($5, discount_pct),
            discount_days = COALESCE($6, discount_days),
            installment_schedule = COALESCE($7, installment_schedule),
            is_active = COALESCE($8, is_active),
            updated_at = NOW()
        WHERE term_id = $1 AND tenant_id = $2
        RETURNING
            term_id, tenant_id, term_code, description, days_due,
            discount_pct, discount_days, installment_schedule,
            idempotency_key, is_active, created_at, updated_at
        "#,
    )
    .bind(term_id)
    .bind(tenant_id)
    .bind(&req.description)
    .bind(req.days_due)
    .bind(req.discount_pct)
    .bind(req.discount_days)
    .bind(&req.installment_schedule)
    .bind(req.is_active)
    .fetch_one(pool)
    .await?;

    Ok(terms)
}

/// Assign payment terms to a vendor bill.
///
/// Computes the due_date, discount_date, and discount_amount from the terms
/// and the bill's invoice_date, then updates the bill row.
pub async fn assign_terms_to_bill(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
    term_id: Uuid,
) -> Result<AssignTermsResult, PaymentTermsError> {
    // Guard: terms must exist and be active
    let terms = get_terms(pool, tenant_id, term_id)
        .await?
        .ok_or(PaymentTermsError::NotFound(term_id))?;

    if !terms.is_active {
        return Err(PaymentTermsError::Validation(
            "Cannot assign inactive payment terms".to_string(),
        ));
    }

    // Guard: bill must exist for this tenant
    let bill_row: Option<(chrono::DateTime<Utc>, i64)> = sqlx::query_as(
        "SELECT invoice_date, total_minor FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let (invoice_date, total_minor) = bill_row.ok_or(PaymentTermsError::BillNotFound(bill_id))?;

    let invoice_naive = invoice_date.date_naive();
    let due_naive = compute_due_date(invoice_naive, terms.days_due);
    let due_date = due_naive
        .and_hms_opt(0, 0, 0)
        .expect("valid time")
        .and_utc();

    let discount_pct_f64: f64 = terms.discount_pct;

    let discount_date = compute_discount_date(invoice_naive, terms.discount_days)
        .map(|d| d.and_hms_opt(0, 0, 0).expect("valid time").and_utc());

    let discount_amount = compute_discount_amount(total_minor, discount_pct_f64);

    // Mutation: update vendor_bills with computed dates
    sqlx::query(
        r#"
        UPDATE vendor_bills
        SET payment_terms_id = $3,
            due_date = $4,
            discount_date = $5,
            discount_amount_minor = $6
        WHERE bill_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .bind(term_id)
    .bind(due_date)
    .bind(discount_date)
    .bind(discount_amount)
    .execute(pool)
    .await?;

    Ok(AssignTermsResult {
        bill_id,
        term_id,
        due_date,
        discount_date,
        discount_amount_minor: discount_amount,
    })
}
