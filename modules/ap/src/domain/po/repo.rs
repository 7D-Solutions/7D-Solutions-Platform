//! PO repository — SQL layer for purchase_orders and po_status tables.
//!
//! All raw SQL for the PO approval mutations lives here.
//! The service layer (approve.rs) calls these functions and owns
//! Guard → Mutation → Outbox orchestration.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{PoError, PurchaseOrder};

// ============================================================================
// Writes (conn-based — called within a transaction)
// ============================================================================

/// SELECT … FOR UPDATE on the purchase_orders row. Locks the row to prevent
/// concurrent approvals.
pub async fn lock_po_for_update(
    conn: &mut PgConnection,
    po_id: Uuid,
    tenant_id: &str,
) -> Result<Option<PurchaseOrder>, PoError> {
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
    .fetch_optional(&mut *conn)
    .await?;
    Ok(po)
}

/// UPDATE purchase_orders SET status = 'approved'. Returns the updated row.
pub async fn approve_po_status(
    conn: &mut PgConnection,
    po_id: Uuid,
    tenant_id: &str,
) -> Result<PurchaseOrder, PoError> {
    let po: PurchaseOrder = sqlx::query_as(
        r#"
        UPDATE purchase_orders
        SET status = 'approved'
        WHERE po_id = $1 AND tenant_id = $2
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(po)
}

/// INSERT an audit entry into po_status for the 'approved' transition.
pub async fn insert_po_status_entry(
    conn: &mut PgConnection,
    po_id: Uuid,
    changed_by: &str,
    changed_at: DateTime<Utc>,
) -> Result<(), PoError> {
    sqlx::query(
        "INSERT INTO po_status (po_id, status, changed_by, changed_at) \
         VALUES ($1, 'approved', $2, $3)",
    )
    .bind(po_id)
    .bind(changed_by)
    .bind(changed_at)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
