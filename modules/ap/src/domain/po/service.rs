//! PO service — Guard → Mutation → Outbox write operations.
//!
//! create_po:        creates a draft PO with lines; emits ap.po_created
//! update_po_lines:  replaces all lines on a draft PO (idempotent)
//!
//! Read queries live in `queries.rs`.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_po_created_envelope, PoCreatedPayload, PoLine as EventPoLine, EVENT_TYPE_PO_CREATED,
};
use crate::outbox::enqueue_event_tx;

use crate::domain::vendors::QualificationStatus;

use super::{
    CreatePoLineRequest, CreatePoRequest, PoError, PoLineRecord, PurchaseOrder,
    PurchaseOrderWithLines, UpdatePoLinesRequest,
};

// ============================================================================
// Writes
// ============================================================================

/// Create a draft PO with line items. Emits `ap.po_created` via the outbox.
///
/// Guard:    vendor must exist and be active for this tenant.
/// Mutation: purchase_orders header + po_lines + po_status audit row.
/// Outbox:   ap.po_created envelope enqueued atomically.
pub async fn create_po(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePoRequest,
    correlation_id: String,
) -> Result<PurchaseOrderWithLines, PoError> {
    req.validate()?;

    // Guard: vendor must exist, be active, and be qualification-eligible
    let vendor_row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT vendor_id, qualification_status FROM vendors \
         WHERE vendor_id = $1 AND tenant_id = $2 AND is_active = TRUE",
    )
    .bind(req.vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    match vendor_row {
        None => return Err(PoError::VendorNotFound(req.vendor_id)),
        Some((_, ref status_str)) => {
            let status = QualificationStatus::from_str(status_str)
                .unwrap_or(QualificationStatus::Unqualified);
            if !status.allows_po() {
                return Err(PoError::VendorNotEligible(req.vendor_id, status_str.clone()));
            }
        }
    }

    let po_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    // PO number: PO-YYYYMMDD-{first 8 hex chars of UUID}
    let po_number = format!(
        "PO-{}-{}",
        now.format("%Y%m%d"),
        &po_id.simple().to_string()[..8].to_uppercase()
    );

    let total_minor: i64 = req.lines.iter().map(|l| l.line_total_minor()).sum();

    let mut tx = pool.begin().await?;

    // Mutation: insert PO header (status = draft)
    let po: PurchaseOrder = sqlx::query_as(
        r#"
        INSERT INTO purchase_orders (
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7, $8, $9)
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .bind(req.vendor_id)
    .bind(&po_number)
    .bind(req.currency.to_uppercase())
    .bind(total_minor)
    .bind(req.created_by.trim())
    .bind(now)
    .bind(req.expected_delivery_date)
    .fetch_one(&mut *tx)
    .await?;

    // Mutation: append draft entry to status audit log
    sqlx::query(
        "INSERT INTO po_status (po_id, status, changed_by, changed_at) VALUES ($1, 'draft', $2, $3)",
    )
    .bind(po_id)
    .bind(req.created_by.trim())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Mutation: insert lines
    let (po_lines, event_lines) = insert_lines(&mut tx, po_id, &req.lines, now).await?;

    // Outbox: ap.po_created
    let payload = PoCreatedPayload {
        po_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: req.vendor_id,
        po_number: po_number.clone(),
        currency: req.currency.to_uppercase(),
        lines: event_lines,
        total_minor,
        created_by: req.created_by.trim().to_string(),
        created_at: now,
        expected_delivery_date: req.expected_delivery_date,
    };

    let envelope = build_po_created_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PO_CREATED,
        "po",
        &po_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(PurchaseOrderWithLines {
        po,
        lines: po_lines,
    })
}

/// Replace all lines on a draft PO (idempotent full replacement).
///
/// Only permitted when PO is in 'draft' status — returns PoError::NotDraft otherwise.
/// Recalculates and stores the new total_minor after replacement.
pub async fn update_po_lines(
    pool: &PgPool,
    tenant_id: &str,
    po_id: Uuid,
    req: &UpdatePoLinesRequest,
) -> Result<PurchaseOrderWithLines, PoError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: PO must exist for this tenant and be in draft
    let po: Option<PurchaseOrder> = sqlx::query_as(
        r#"
        SELECT po_id, tenant_id, vendor_id, po_number, currency,
               total_minor, status, created_by, created_at, expected_delivery_date
        FROM purchase_orders
        WHERE po_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let po = po.ok_or(PoError::NotFound(po_id))?;

    if po.status != "draft" {
        return Err(PoError::NotDraft(po.status.clone()));
    }

    let now = Utc::now();
    let total_minor: i64 = req.lines.iter().map(|l| l.line_total_minor()).sum();

    // Mutation: replace all lines (delete + re-insert = idempotent)
    sqlx::query("DELETE FROM po_lines WHERE po_id = $1")
        .bind(po_id)
        .execute(&mut *tx)
        .await?;

    let (new_lines, _) = insert_lines(&mut tx, po_id, &req.lines, now).await?;

    // Mutation: update PO total to reflect new lines
    let updated_po: PurchaseOrder = sqlx::query_as(
        r#"
        UPDATE purchase_orders
        SET total_minor = $1
        WHERE po_id = $2 AND tenant_id = $3
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(total_minor)
    .bind(po_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(PurchaseOrderWithLines {
        po: updated_po,
        lines: new_lines,
    })
}

// ============================================================================
// Helpers
// ============================================================================

/// Insert PO lines within a caller-owned transaction.
/// Returns (DB records, event lines).
async fn insert_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    po_id: Uuid,
    lines: &[CreatePoLineRequest],
    now: DateTime<Utc>,
) -> Result<(Vec<PoLineRecord>, Vec<EventPoLine>), PoError> {
    let mut db_lines = Vec::with_capacity(lines.len());
    let mut event_lines = Vec::with_capacity(lines.len());

    for line_req in lines {
        let line_id = Uuid::new_v4();
        let description = line_req.effective_description();
        let line_total = line_req.line_total_minor();

        let line: PoLineRecord = sqlx::query_as(
            r#"
            INSERT INTO po_lines (
                line_id, po_id, item_id, description, quantity, unit_of_measure,
                unit_price_minor, line_total_minor, gl_account_code, created_at
            )
            VALUES ($1, $2, $3, $4, $5::NUMERIC, $6, $7, $8, $9, $10)
            RETURNING
                line_id, po_id, item_id, description,
                quantity::FLOAT8 AS quantity,
                unit_of_measure, unit_price_minor, line_total_minor,
                gl_account_code, created_at
            "#,
        )
        .bind(line_id)
        .bind(po_id)
        .bind(line_req.item_id)
        .bind(&description)
        .bind(line_req.quantity)
        .bind(&line_req.unit_of_measure)
        .bind(line_req.unit_price_minor)
        .bind(line_total)
        .bind(&line_req.gl_account_code)
        .bind(now)
        .fetch_one(&mut **tx)
        .await?;

        event_lines.push(EventPoLine {
            line_id,
            item_id: line_req.item_id,
            description: description.clone(),
            quantity: line_req.quantity,
            unit_of_measure: line_req.unit_of_measure.clone(),
            unit_price_minor: line_req.unit_price_minor,
            gl_account_code: line_req.gl_account_code.clone(),
        });
        db_lines.push(line);
    }

    Ok((db_lines, event_lines))
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
