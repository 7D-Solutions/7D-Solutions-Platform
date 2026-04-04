//! Bill repository — SQL layer for vendor_bills and bill_lines.
//!
//! All raw SQL lives here. The service layer calls these functions
//! for persistence and delegates business logic (guards, due-date
//! derivation, outbox orchestration) to its own methods.

use chrono::DateTime;
use chrono::Utc;
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{BillHeaderRow, BillLineGlRow};
use super::{BillError, BillLineRecord, VendorBill};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single bill header by ID + tenant. Returns None if not found.
pub async fn fetch_bill(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Option<VendorBill>, BillError> {
    let bill = sqlx::query_as(
        r#"
        SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
               total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
               entered_by, entered_at
        FROM vendor_bills
        WHERE bill_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(bill)
}

/// Fetch all line items for a bill, ordered by creation time.
pub async fn fetch_bill_lines(
    pool: &PgPool,
    bill_id: Uuid,
) -> Result<Vec<BillLineRecord>, BillError> {
    let lines = sqlx::query_as(
        r#"
        SELECT line_id, bill_id, description, quantity, unit_price_minor,
               line_total_minor, gl_account_code, po_line_id, created_at
        FROM bill_lines
        WHERE bill_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(bill_id)
    .fetch_all(pool)
    .await?;

    Ok(lines)
}

/// List bills for a tenant with optional vendor filter and voided inclusion.
pub async fn list_bills(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Option<Uuid>,
    include_voided: bool,
) -> Result<Vec<VendorBill>, BillError> {
    let bills = match (vendor_id, include_voided) {
        (Some(vid), true) => {
            sqlx::query_as::<_, VendorBill>(
                r#"
                SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
                       total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
                       entered_by, entered_at
                FROM vendor_bills
                WHERE tenant_id = $1 AND vendor_id = $2
                ORDER BY invoice_date DESC
                "#,
            )
            .bind(tenant_id)
            .bind(vid)
            .fetch_all(pool)
            .await?
        }
        (Some(vid), false) => {
            sqlx::query_as::<_, VendorBill>(
                r#"
                SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
                       total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
                       entered_by, entered_at
                FROM vendor_bills
                WHERE tenant_id = $1 AND vendor_id = $2 AND status != 'voided'
                ORDER BY invoice_date DESC
                "#,
            )
            .bind(tenant_id)
            .bind(vid)
            .fetch_all(pool)
            .await?
        }
        (None, true) => {
            sqlx::query_as::<_, VendorBill>(
                r#"
                SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
                       total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
                       entered_by, entered_at
                FROM vendor_bills
                WHERE tenant_id = $1
                ORDER BY invoice_date DESC
                "#,
            )
            .bind(tenant_id)
            .fetch_all(pool)
            .await?
        }
        (None, false) => {
            sqlx::query_as::<_, VendorBill>(
                r#"
                SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
                       total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
                       entered_by, entered_at
                FROM vendor_bills
                WHERE tenant_id = $1 AND status != 'voided'
                ORDER BY invoice_date DESC
                "#,
            )
            .bind(tenant_id)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(bills)
}

// ============================================================================
// Guard queries
// ============================================================================

/// Fetch an active vendor's ID and payment terms. Returns None if not found or inactive.
pub async fn fetch_active_vendor(
    pool: &PgPool,
    vendor_id: Uuid,
    tenant_id: &str,
) -> Result<Option<(Uuid, i32)>, BillError> {
    let row = sqlx::query_as(
        r#"
        SELECT vendor_id, payment_terms_days
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2 AND is_active = TRUE
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

// ============================================================================
// Writes (within a transaction)
// ============================================================================

/// Insert a bill header row. Returns the inserted record.
///
/// Maps the 23505 unique-violation to `BillError::DuplicateInvoice`.
pub async fn insert_bill(
    conn: &mut PgConnection,
    bill_id: Uuid,
    tenant_id: &str,
    vendor_id: Uuid,
    vendor_invoice_ref: &str,
    currency: &str,
    total_minor: i64,
    tax_minor: Option<i64>,
    invoice_date: DateTime<Utc>,
    due_date: DateTime<Utc>,
    fx_rate_id: Option<Uuid>,
    entered_by: &str,
    entered_at: DateTime<Utc>,
) -> Result<VendorBill, BillError> {
    let bill: VendorBill = sqlx::query_as(
        r#"
        INSERT INTO vendor_bills (
            bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
            entered_by, entered_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'open', $10, $11, $12)
        RETURNING
            bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
            entered_by, entered_at
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .bind(vendor_id)
    .bind(vendor_invoice_ref)
    .bind(currency)
    .bind(total_minor)
    .bind(tax_minor)
    .bind(invoice_date)
    .bind(due_date)
    .bind(fx_rate_id)
    .bind(entered_by)
    .bind(entered_at)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db) = e {
            if db.code().as_deref() == Some("23505") {
                return BillError::DuplicateInvoice(vendor_invoice_ref.to_string());
            }
        }
        BillError::Database(e)
    })?;

    Ok(bill)
}

/// Insert a single bill line row. Returns the inserted record.
pub async fn insert_bill_line(
    conn: &mut PgConnection,
    line_id: Uuid,
    bill_id: Uuid,
    description: &str,
    quantity: f64,
    unit_price_minor: i64,
    line_total_minor: i64,
    gl_account_code: &str,
    po_line_id: Option<Uuid>,
    created_at: DateTime<Utc>,
) -> Result<BillLineRecord, BillError> {
    let line: BillLineRecord = sqlx::query_as(
        r#"
        INSERT INTO bill_lines (
            line_id, bill_id, description, quantity, unit_price_minor,
            line_total_minor, gl_account_code, po_line_id, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING
            line_id, bill_id, description, quantity, unit_price_minor,
            line_total_minor, gl_account_code, po_line_id, created_at
        "#,
    )
    .bind(line_id)
    .bind(bill_id)
    .bind(description)
    .bind(quantity)
    .bind(unit_price_minor)
    .bind(line_total_minor)
    .bind(gl_account_code)
    .bind(po_line_id)
    .bind(created_at)
    .fetch_one(&mut *conn)
    .await?;

    Ok(line)
}

// ============================================================================
// Approve queries (within a transaction)
// ============================================================================

/// SELECT bill header FOR UPDATE. Locks the bill during approval to prevent
/// concurrent mutations.
pub async fn lock_bill_header(
    conn: &mut PgConnection,
    bill_id: Uuid,
    tenant_id: &str,
) -> Result<Option<BillHeaderRow>, BillError> {
    let row: Option<BillHeaderRow> = sqlx::query_as(
        r#"
        SELECT vendor_id, vendor_invoice_ref, total_minor, currency, due_date, status,
               fx_rate_id
        FROM vendor_bills
        WHERE bill_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row)
}

/// UPDATE vendor_bills SET status = 'approved'. Returns the updated bill.
pub async fn approve_bill_status(
    conn: &mut PgConnection,
    bill_id: Uuid,
    tenant_id: &str,
) -> Result<VendorBill, BillError> {
    let bill: VendorBill = sqlx::query_as(
        r#"
        UPDATE vendor_bills
        SET status = 'approved'
        WHERE bill_id = $1 AND tenant_id = $2
        RETURNING
            bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
            entered_by, entered_at
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(bill)
}

/// Fetch bill lines for GL posting (ordered by line_id).
pub async fn fetch_bill_gl_lines(
    conn: &mut PgConnection,
    bill_id: Uuid,
) -> Result<Vec<BillLineGlRow>, BillError> {
    let rows: Vec<BillLineGlRow> = sqlx::query_as(
        r#"
        SELECT line_id, gl_account_code, line_total_minor, po_line_id
        FROM bill_lines
        WHERE bill_id = $1
        ORDER BY line_id
        "#,
    )
    .bind(bill_id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows)
}
