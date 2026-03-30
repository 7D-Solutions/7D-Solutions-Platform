//! PO read queries — get_po, list_pos, fetch_lines.

use sqlx::PgPool;
use uuid::Uuid;

use super::{PoError, PoLineRecord, PurchaseOrder, PurchaseOrderWithLines};

/// Fetch a single PO with its lines. Returns None if not found for this tenant.
pub async fn get_po(
    pool: &PgPool,
    tenant_id: &str,
    po_id: Uuid,
) -> Result<Option<PurchaseOrderWithLines>, PoError> {
    let po: Option<PurchaseOrder> = sqlx::query_as(
        r#"
        SELECT po_id, tenant_id, vendor_id, po_number, currency,
               total_minor, status, created_by, created_at, expected_delivery_date
        FROM purchase_orders
        WHERE po_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let Some(po) = po else {
        return Ok(None);
    };

    let lines = fetch_lines(pool, po_id).await?;
    Ok(Some(PurchaseOrderWithLines { po, lines }))
}

/// List PO headers for a tenant with optional vendor_id and status filters.
pub async fn list_pos(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Option<Uuid>,
    status: Option<&str>,
) -> Result<Vec<PurchaseOrder>, PoError> {
    let pos = match (vendor_id, status) {
        (Some(vid), Some(s)) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND vendor_id = $2 AND status = $3
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id)
            .bind(vid)
            .bind(s)
            .fetch_all(pool)
            .await?
        }
        (Some(vid), None) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND vendor_id = $2
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id)
            .bind(vid)
            .fetch_all(pool)
            .await?
        }
        (None, Some(s)) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND status = $2
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id)
            .bind(s)
            .fetch_all(pool)
            .await?
        }
        (None, None) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(pos)
}

pub(super) async fn fetch_lines(
    pool: &PgPool,
    po_id: Uuid,
) -> Result<Vec<PoLineRecord>, PoError> {
    let lines = sqlx::query_as::<_, PoLineRecord>(
        r#"
        SELECT line_id, po_id, description,
               quantity::FLOAT8 AS quantity,
               unit_of_measure, unit_price_minor, line_total_minor,
               gl_account_code, created_at
        FROM po_lines
        WHERE po_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(po_id)
    .fetch_all(pool)
    .await?;
    Ok(lines)
}
