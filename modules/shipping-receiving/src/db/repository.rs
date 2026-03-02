//! Shipment DB repository — all SQL for shipments and shipment_lines.
//!
//! Every function takes &PgPool (or &mut Transaction) + tenant_id.
//! No business logic here — pure data access.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::shipments::service::Shipment;
use crate::domain::shipments::types::LineQty;
use crate::http::shipments::ShipmentLineRow;

// ── Shipment CRUD ────────────────────────────────────────────

pub struct ShipmentRepository;

/// Parameters for inserting a new shipment.
pub struct InsertShipmentParams {
    pub tenant_id: Uuid,
    pub direction: String,
    pub status: String,
    pub carrier_party_id: Option<Uuid>,
    pub tracking_number: Option<String>,
    pub freight_cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub expected_arrival_date: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub source_ref_type: Option<String>,
    pub source_ref_id: Option<Uuid>,
}

/// Parameters for inserting a shipment line.
pub struct InsertLineParams {
    pub tenant_id: Uuid,
    pub shipment_id: Uuid,
    pub sku: Option<String>,
    pub uom: Option<String>,
    pub warehouse_id: Option<Uuid>,
    pub qty_expected: i64,
    pub source_ref_type: Option<String>,
    pub source_ref_id: Option<Uuid>,
    pub po_id: Option<Uuid>,
    pub po_line_id: Option<Uuid>,
}

impl ShipmentRepository {
    // ── Shipment queries ─────────────────────────────────────

    pub async fn insert_shipment(
        pool: &PgPool,
        p: &InsertShipmentParams,
    ) -> Result<Shipment, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            INSERT INTO shipments (tenant_id, direction, status, carrier_party_id,
                tracking_number, freight_cost_minor, currency, expected_arrival_date, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(p.tenant_id)
        .bind(&p.direction)
        .bind(&p.status)
        .bind(p.carrier_party_id)
        .bind(&p.tracking_number)
        .bind(p.freight_cost_minor)
        .bind(&p.currency)
        .bind(p.expected_arrival_date)
        .bind(p.created_by)
        .fetch_one(pool)
        .await
    }

    pub async fn insert_shipment_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        p: &InsertShipmentParams,
    ) -> Result<Shipment, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            INSERT INTO shipments (tenant_id, direction, status, carrier_party_id,
                tracking_number, freight_cost_minor, currency, expected_arrival_date, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(p.tenant_id)
        .bind(&p.direction)
        .bind(&p.status)
        .bind(p.carrier_party_id)
        .bind(&p.tracking_number)
        .bind(p.freight_cost_minor)
        .bind(&p.currency)
        .bind(p.expected_arrival_date)
        .bind(p.created_by)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn get_shipment(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Shipment>, sqlx::Error> {
        sqlx::query_as::<_, Shipment>("SELECT * FROM shipments WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await
    }

    pub async fn get_shipment_for_update(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Shipment>, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            "SELECT * FROM shipments WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut **tx)
        .await
    }

    pub async fn update_shipment_status(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: Uuid,
        tenant_id: Uuid,
        status: &str,
        arrived_at: Option<DateTime<Utc>>,
        shipped_at: Option<DateTime<Utc>>,
        delivered_at: Option<DateTime<Utc>>,
        closed_at: Option<DateTime<Utc>>,
    ) -> Result<Shipment, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            UPDATE shipments SET
                status       = $3,
                arrived_at   = COALESCE($4, arrived_at),
                shipped_at   = COALESCE($5, shipped_at),
                delivered_at = COALESCE($6, delivered_at),
                closed_at    = COALESCE($7, closed_at),
                updated_at   = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(status)
        .bind(arrived_at)
        .bind(shipped_at)
        .bind(delivered_at)
        .bind(closed_at)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn list_shipments(
        pool: &PgPool,
        tenant_id: Uuid,
        direction: Option<&str>,
        status: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Shipment>, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            SELECT * FROM shipments
            WHERE tenant_id = $1
              AND ($2::text IS NULL OR direction = $2)
              AND ($3::text IS NULL OR status = $3)
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(tenant_id)
        .bind(direction)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
    }

    pub async fn find_by_source_ref(
        pool: &PgPool,
        tenant_id: Uuid,
        source_ref_type: &str,
        source_ref_id: Uuid,
    ) -> Result<Vec<Shipment>, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            SELECT s.* FROM shipments s
            INNER JOIN shipment_lines sl ON s.id = sl.shipment_id
            WHERE s.tenant_id = $1
              AND sl.source_ref_type = $2
              AND sl.source_ref_id = $3
            GROUP BY s.id
            "#,
        )
        .bind(tenant_id)
        .bind(source_ref_type)
        .bind(source_ref_id)
        .fetch_all(pool)
        .await
    }

    // ── Line queries ─────────────────────────────────────────

    pub async fn insert_line(
        pool: &PgPool,
        p: &InsertLineParams,
    ) -> Result<ShipmentLineRow, sqlx::Error> {
        sqlx::query_as::<_, ShipmentLineRow>(
            r#"
            INSERT INTO shipment_lines (tenant_id, shipment_id, sku, uom, warehouse_id,
                qty_expected, source_ref_type, source_ref_id, po_id, po_line_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING *
            "#,
        )
        .bind(p.tenant_id)
        .bind(p.shipment_id)
        .bind(&p.sku)
        .bind(&p.uom)
        .bind(p.warehouse_id)
        .bind(p.qty_expected)
        .bind(&p.source_ref_type)
        .bind(p.source_ref_id)
        .bind(p.po_id)
        .bind(p.po_line_id)
        .fetch_one(pool)
        .await
    }

    pub async fn insert_line_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        p: &InsertLineParams,
    ) -> Result<ShipmentLineRow, sqlx::Error> {
        sqlx::query_as::<_, ShipmentLineRow>(
            r#"
            INSERT INTO shipment_lines (tenant_id, shipment_id, sku, uom, warehouse_id,
                qty_expected, source_ref_type, source_ref_id, po_id, po_line_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            RETURNING *
            "#,
        )
        .bind(p.tenant_id)
        .bind(p.shipment_id)
        .bind(&p.sku)
        .bind(&p.uom)
        .bind(p.warehouse_id)
        .bind(p.qty_expected)
        .bind(&p.source_ref_type)
        .bind(p.source_ref_id)
        .bind(p.po_id)
        .bind(p.po_line_id)
        .fetch_one(&mut **tx)
        .await
    }

    pub async fn get_lines_for_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<ShipmentLineRow>, sqlx::Error> {
        sqlx::query_as::<_, ShipmentLineRow>(
            "SELECT * FROM shipment_lines WHERE shipment_id = $1 AND tenant_id = $2 ORDER BY created_at",
        )
        .bind(shipment_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }

    pub async fn get_line_qtys_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<LineQty>, sqlx::Error> {
        let rows: Vec<LineQtyRow> = sqlx::query_as(
            r#"
            SELECT id, qty_expected, qty_shipped, qty_received, qty_accepted, qty_rejected
            FROM shipment_lines
            WHERE shipment_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(shipment_id)
        .bind(tenant_id)
        .fetch_all(&mut **tx)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| LineQty {
                line_id: r.id,
                qty_expected: r.qty_expected,
                qty_shipped: r.qty_shipped,
                qty_received: r.qty_received,
                qty_accepted: r.qty_accepted,
                qty_rejected: r.qty_rejected,
            })
            .collect())
    }

    // ── Inventory line helpers ─────────────────────────────────

    /// Fetch lines for inventory processing within a transaction.
    /// Returns id, warehouse_id, inventory_ref_id, qty_accepted, qty_shipped.
    pub async fn get_inventory_lines_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<InventoryLineRow>, sqlx::Error> {
        sqlx::query_as::<_, InventoryLineRow>(
            r#"
            SELECT id, warehouse_id, inventory_ref_id, qty_accepted, qty_shipped
            FROM shipment_lines
            WHERE shipment_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(shipment_id)
        .bind(tenant_id)
        .fetch_all(&mut **tx)
        .await
    }

    /// Set the inventory_ref_id on a shipment line within a transaction.
    pub async fn set_inventory_ref_id_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        line_id: Uuid,
        tenant_id: Uuid,
        inventory_ref_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE shipment_lines
            SET inventory_ref_id = $3, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(line_id)
        .bind(tenant_id)
        .bind(inventory_ref_id)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    // ── Ref-linkage queries ────────────────────────────────────

    /// Find shipments that have at least one line referencing the given PO.
    pub async fn find_shipments_by_po(
        pool: &PgPool,
        tenant_id: Uuid,
        po_id: Uuid,
    ) -> Result<Vec<Shipment>, sqlx::Error> {
        sqlx::query_as::<_, Shipment>(
            r#"
            SELECT DISTINCT s.* FROM shipments s
            INNER JOIN shipment_lines sl ON s.id = sl.shipment_id AND sl.tenant_id = s.tenant_id
            WHERE s.tenant_id = $1
              AND sl.po_id = $2
            ORDER BY s.created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(po_id)
        .fetch_all(pool)
        .await
    }

    /// Find shipment lines for a specific PO line.
    pub async fn find_lines_by_po_line(
        pool: &PgPool,
        tenant_id: Uuid,
        po_line_id: Uuid,
    ) -> Result<Vec<ShipmentLineRow>, sqlx::Error> {
        sqlx::query_as::<_, ShipmentLineRow>(
            r#"
            SELECT * FROM shipment_lines
            WHERE tenant_id = $1 AND po_line_id = $2
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(po_line_id)
        .fetch_all(pool)
        .await
    }

    // ── Idempotency ──────────────────────────────────────────

    /// Check if an event has already been processed. Returns true if so.
    pub async fn is_event_processed(pool: &PgPool, event_id: Uuid) -> Result<bool, sqlx::Error> {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT 1 as x FROM sr_processed_events WHERE event_id = $1")
                .bind(event_id)
                .fetch_optional(pool)
                .await?;
        Ok(row.is_some())
    }

    /// Mark an event as processed (idempotent via ON CONFLICT DO NOTHING).
    pub async fn mark_event_processed_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event_id: Uuid,
        event_type: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            INSERT INTO sr_processed_events (event_id, event_type)
            VALUES ($1, $2)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(event_type)
        .execute(&mut **tx)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

/// Internal row type for line quantity queries.
#[derive(sqlx::FromRow)]
struct LineQtyRow {
    id: Uuid,
    qty_expected: i64,
    qty_shipped: i64,
    qty_received: i64,
    qty_accepted: i64,
    qty_rejected: i64,
}

/// Row type for inventory-relevant line data within a transaction.
#[derive(Debug, sqlx::FromRow)]
pub struct InventoryLineRow {
    pub id: Uuid,
    pub warehouse_id: Option<Uuid>,
    pub inventory_ref_id: Option<Uuid>,
    pub qty_accepted: i64,
    pub qty_shipped: i64,
}
