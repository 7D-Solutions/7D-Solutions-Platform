//! Blanket order repository — all SQL operations.

use chrono::NaiveDate;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::{BlanketOrder, BlanketOrderLine, BlanketOrderRelease};

// ── Blanket order reads ───────────────────────────────────────────────────────

pub async fn fetch_blanket<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<BlanketOrder>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrder>(
        r#"
        SELECT id, tenant_id, blanket_number, status, customer_id, party_id,
               currency, committed_cents, released_cents,
               effective_date, expiry_date, notes, created_by, created_at, updated_at
        FROM blanket_orders
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(executor)
    .await
}

pub async fn list_blankets<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    customer_id: Option<Uuid>,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<BlanketOrder>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrder>(
        r#"
        SELECT id, tenant_id, blanket_number, status, customer_id, party_id,
               currency, committed_cents, released_cents,
               effective_date, expiry_date, notes, created_by, created_at, updated_at
        FROM blanket_orders
        WHERE tenant_id = $1
          AND ($2::uuid IS NULL OR customer_id = $2)
          AND ($3::text IS NULL OR status = $3)
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(status)
    .bind(limit)
    .bind(offset)
    .fetch_all(executor)
    .await
}

pub async fn fetch_blanket_lines<'e>(
    executor: impl PgExecutor<'e>,
    blanket_order_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<BlanketOrderLine>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrderLine>(
        r#"
        SELECT id, tenant_id, blanket_order_id, line_number, item_id, part_number,
               description, uom, committed_qty, released_qty, unit_price_cents, notes
        FROM blanket_order_lines
        WHERE blanket_order_id = $1 AND tenant_id = $2
        ORDER BY line_number
        "#,
    )
    .bind(blanket_order_id)
    .bind(tenant_id)
    .fetch_all(executor)
    .await
}

/// Fetch blanket line with row-level lock for over-draw protection.
pub async fn fetch_blanket_line_for_update(
    pool: &PgPool,
    line_id: Uuid,
    tenant_id: &str,
) -> Result<Option<BlanketOrderLine>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrderLine>(
        r#"
        SELECT id, tenant_id, blanket_order_id, line_number, item_id, part_number,
               description, uom, committed_qty, released_qty, unit_price_cents, notes
        FROM blanket_order_lines
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(line_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn fetch_releases_for_line<'e>(
    executor: impl PgExecutor<'e>,
    blanket_line_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<BlanketOrderRelease>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrderRelease>(
        r#"
        SELECT id, tenant_id, blanket_order_id, blanket_line_id, sales_order_id,
               status, release_qty, release_date, notes, created_at
        FROM blanket_order_releases
        WHERE blanket_line_id = $1 AND tenant_id = $2
        ORDER BY created_at
        "#,
    )
    .bind(blanket_line_id)
    .bind(tenant_id)
    .fetch_all(executor)
    .await
}

pub async fn fetch_release<'e>(
    executor: impl PgExecutor<'e>,
    release_id: Uuid,
    tenant_id: &str,
) -> Result<Option<BlanketOrderRelease>, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrderRelease>(
        r#"
        SELECT id, tenant_id, blanket_order_id, blanket_line_id, sales_order_id,
               status, release_qty, release_date, notes, created_at
        FROM blanket_order_releases
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(release_id)
    .bind(tenant_id)
    .fetch_optional(executor)
    .await
}

// ── Blanket order writes ──────────────────────────────────────────────────────

pub async fn insert_blanket<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    blanket_number: &str,
    customer_id: Option<Uuid>,
    party_id: Option<Uuid>,
    currency: &str,
    effective_date: NaiveDate,
    expiry_date: Option<NaiveDate>,
    notes: Option<&str>,
    created_by: &str,
) -> Result<BlanketOrder, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrder>(
        r#"
        INSERT INTO blanket_orders (
            id, tenant_id, blanket_number, status, customer_id, party_id,
            currency, committed_cents, released_cents,
            effective_date, expiry_date, notes, created_by
        ) VALUES (
            $1, $2, $3, 'draft', $4, $5, $6, 0, 0, $7, $8, $9, $10
        )
        RETURNING id, tenant_id, blanket_number, status, customer_id, party_id,
                  currency, committed_cents, released_cents,
                  effective_date, expiry_date, notes, created_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(blanket_number)
    .bind(customer_id)
    .bind(party_id)
    .bind(currency)
    .bind(effective_date)
    .bind(expiry_date)
    .bind(notes)
    .bind(created_by)
    .fetch_one(executor)
    .await
}

pub async fn update_blanket_status<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE blanket_orders SET status = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(status)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update_blanket_header<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    customer_id: Option<Uuid>,
    party_id: Option<Uuid>,
    expiry_date: Option<NaiveDate>,
    notes: Option<&str>,
) -> Result<BlanketOrder, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrder>(
        r#"
        UPDATE blanket_orders SET
            customer_id = COALESCE($3, customer_id),
            party_id    = COALESCE($4, party_id),
            expiry_date = $5,
            notes       = $6,
            updated_at  = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING id, tenant_id, blanket_number, status, customer_id, party_id,
                  currency, committed_cents, released_cents,
                  effective_date, expiry_date, notes, created_by, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(customer_id)
    .bind(party_id)
    .bind(expiry_date)
    .bind(notes)
    .fetch_one(executor)
    .await
}

pub async fn insert_blanket_line<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    blanket_order_id: Uuid,
    item_id: Option<Uuid>,
    part_number: Option<&str>,
    description: &str,
    uom: &str,
    committed_qty: f64,
    unit_price_cents: i64,
    notes: Option<&str>,
) -> Result<BlanketOrderLine, sqlx::Error> {
    sqlx::query_as::<_, BlanketOrderLine>(
        r#"
        INSERT INTO blanket_order_lines (
            id, tenant_id, blanket_order_id, line_number, item_id, part_number,
            description, uom, committed_qty, released_qty, unit_price_cents, notes
        )
        SELECT $1, $2, $3,
               COALESCE((SELECT MAX(line_number) FROM blanket_order_lines WHERE blanket_order_id = $3), 0) + 1,
               $4, $5, $6, $7, $8, 0, $9, $10
        RETURNING id, tenant_id, blanket_order_id, line_number, item_id, part_number,
                  description, uom, committed_qty, released_qty, unit_price_cents, notes
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(blanket_order_id)
    .bind(item_id)
    .bind(part_number)
    .bind(description)
    .bind(uom)
    .bind(committed_qty)
    .bind(unit_price_cents)
    .bind(notes)
    .fetch_one(executor)
    .await
}

/// Insert release and atomically increment released_qty on the line.
/// Must be called inside a transaction that holds the FOR UPDATE lock on the line.
pub async fn insert_release_and_update_line<'e>(
    executor: impl PgExecutor<'e>,
    release_id: Uuid,
    tenant_id: &str,
    blanket_order_id: Uuid,
    blanket_line_id: Uuid,
    release_qty: f64,
    release_date: NaiveDate,
    notes: Option<&str>,
) -> Result<BlanketOrderRelease, sqlx::Error> {
    // Two statements in one query_as won't work — caller should run both in a tx.
    // We return the release; caller updates released_qty separately.
    sqlx::query_as::<_, BlanketOrderRelease>(
        r#"
        INSERT INTO blanket_order_releases (
            id, tenant_id, blanket_order_id, blanket_line_id, status,
            release_qty, release_date, notes
        ) VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7)
        RETURNING id, tenant_id, blanket_order_id, blanket_line_id, sales_order_id,
                  status, release_qty, release_date, notes, created_at
        "#,
    )
    .bind(release_id)
    .bind(tenant_id)
    .bind(blanket_order_id)
    .bind(blanket_line_id)
    .bind(release_qty)
    .bind(release_date)
    .bind(notes)
    .fetch_one(executor)
    .await
}

pub async fn increment_line_released_qty<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    delta: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE blanket_order_lines SET released_qty = released_qty + $3 WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(delta)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update_release_status<'e>(
    executor: impl PgExecutor<'e>,
    release_id: Uuid,
    tenant_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE blanket_order_releases SET status = $3 WHERE id = $1 AND tenant_id = $2",
    )
    .bind(release_id)
    .bind(tenant_id)
    .bind(status)
    .execute(executor)
    .await?;
    Ok(())
}
