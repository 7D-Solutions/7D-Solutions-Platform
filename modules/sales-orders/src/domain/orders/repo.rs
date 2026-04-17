//! Sales order repository — all SQL operations.

use chrono::NaiveDate;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::{SalesOrder, SalesOrderLine};

// ── Guard reads ───────────────────────────────────────────────────────────────

pub async fn fetch_order_for_mutation<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<SalesOrder>, sqlx::Error> {
    sqlx::query_as::<_, SalesOrder>(
        r#"
        SELECT id, tenant_id, order_number, status, customer_id, party_id,
               currency, subtotal_cents, tax_cents, total_cents,
               order_date, required_date, promised_date, external_quote_ref,
               blanket_order_id, blanket_release_id, notes, created_by,
               created_at, updated_at
        FROM sales_orders
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(executor)
    .await
}

// ── Query reads ───────────────────────────────────────────────────────────────

pub async fn fetch_lines_for_order<'e>(
    executor: impl PgExecutor<'e>,
    sales_order_id: Uuid,
    tenant_id: &str,
) -> Result<Vec<SalesOrderLine>, sqlx::Error> {
    sqlx::query_as::<_, SalesOrderLine>(
        r#"
        SELECT id, tenant_id, sales_order_id, line_number, item_id, part_number,
               description, uom, quantity, unit_price_cents, line_total_cents,
               required_date, promised_date, shipped_qty, warehouse_id,
               reservation_id, invoiced_at, notes
        FROM sales_order_lines
        WHERE sales_order_id = $1 AND tenant_id = $2
        ORDER BY line_number
        "#,
    )
    .bind(sales_order_id)
    .bind(tenant_id)
    .fetch_all(executor)
    .await
}

pub async fn list_orders<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    customer_id: Option<Uuid>,
    status: Option<&str>,
    blanket_order_id: Option<Uuid>,
    from_date: Option<NaiveDate>,
    to_date: Option<NaiveDate>,
    limit: i64,
    offset: i64,
) -> Result<Vec<SalesOrder>, sqlx::Error> {
    sqlx::query_as::<_, SalesOrder>(
        r#"
        SELECT id, tenant_id, order_number, status, customer_id, party_id,
               currency, subtotal_cents, tax_cents, total_cents,
               order_date, required_date, promised_date, external_quote_ref,
               blanket_order_id, blanket_release_id, notes, created_by,
               created_at, updated_at
        FROM sales_orders
        WHERE tenant_id = $1
          AND ($2::uuid IS NULL OR customer_id = $2)
          AND ($3::text IS NULL OR status = $3)
          AND ($4::uuid IS NULL OR blanket_order_id = $4)
          AND ($5::date IS NULL OR order_date >= $5)
          AND ($6::date IS NULL OR order_date <= $6)
        ORDER BY created_at DESC
        LIMIT $7 OFFSET $8
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(status)
    .bind(blanket_order_id)
    .bind(from_date)
    .bind(to_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(executor)
    .await
}

pub async fn count_orders<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    customer_id: Option<Uuid>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM sales_orders
        WHERE tenant_id = $1
          AND ($2::uuid IS NULL OR customer_id = $2)
          AND ($3::text IS NULL OR status = $3)
        "#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(status)
    .fetch_one(executor)
    .await?;
    Ok(count)
}

// ── Writes ────────────────────────────────────────────────────────────────────

pub async fn insert_order<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    order_number: &str,
    customer_id: Option<Uuid>,
    party_id: Option<Uuid>,
    currency: &str,
    order_date: NaiveDate,
    required_date: Option<NaiveDate>,
    promised_date: Option<NaiveDate>,
    external_quote_ref: Option<&str>,
    blanket_order_id: Option<Uuid>,
    blanket_release_id: Option<Uuid>,
    notes: Option<&str>,
    created_by: &str,
) -> Result<SalesOrder, sqlx::Error> {
    sqlx::query_as::<_, SalesOrder>(
        r#"
        INSERT INTO sales_orders (
            id, tenant_id, order_number, status, customer_id, party_id,
            currency, subtotal_cents, tax_cents, total_cents,
            order_date, required_date, promised_date, external_quote_ref,
            blanket_order_id, blanket_release_id, notes, created_by
        ) VALUES (
            $1, $2, $3, 'draft', $4, $5,
            $6, 0, 0, 0,
            $7, $8, $9, $10,
            $11, $12, $13, $14
        )
        RETURNING id, tenant_id, order_number, status, customer_id, party_id,
                  currency, subtotal_cents, tax_cents, total_cents,
                  order_date, required_date, promised_date, external_quote_ref,
                  blanket_order_id, blanket_release_id, notes, created_by,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(order_number)
    .bind(customer_id)
    .bind(party_id)
    .bind(currency)
    .bind(order_date)
    .bind(required_date)
    .bind(promised_date)
    .bind(external_quote_ref)
    .bind(blanket_order_id)
    .bind(blanket_release_id)
    .bind(notes)
    .bind(created_by)
    .fetch_one(executor)
    .await
}

pub async fn update_order_header<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    customer_id: Option<Uuid>,
    party_id: Option<Uuid>,
    required_date: Option<NaiveDate>,
    promised_date: Option<NaiveDate>,
    external_quote_ref: Option<&str>,
    notes: Option<&str>,
    tax_cents: i64,
    subtotal_cents: i64,
    total_cents: i64,
) -> Result<SalesOrder, sqlx::Error> {
    sqlx::query_as::<_, SalesOrder>(
        r#"
        UPDATE sales_orders SET
            customer_id    = COALESCE($3, customer_id),
            party_id       = COALESCE($4, party_id),
            required_date  = $5,
            promised_date  = $6,
            external_quote_ref = $7,
            notes          = $8,
            tax_cents      = $9,
            subtotal_cents = $10,
            total_cents    = $11,
            updated_at     = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING id, tenant_id, order_number, status, customer_id, party_id,
                  currency, subtotal_cents, tax_cents, total_cents,
                  order_date, required_date, promised_date, external_quote_ref,
                  blanket_order_id, blanket_release_id, notes, created_by,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(customer_id)
    .bind(party_id)
    .bind(required_date)
    .bind(promised_date)
    .bind(external_quote_ref)
    .bind(notes)
    .bind(tax_cents)
    .bind(subtotal_cents)
    .bind(total_cents)
    .fetch_one(executor)
    .await
}

pub async fn update_order_status<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sales_orders SET status = $3, updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(status)
    .execute(executor)
    .await?;
    Ok(())
}

/// Recompute header totals from line sums and persist.
pub async fn recompute_and_save_totals(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    tax_cents: i64,
) -> Result<(i64, i64), sqlx::Error> {
    let row: (i64, i64) = sqlx::query_as(
        r#"
        WITH line_sum AS (
            SELECT COALESCE(SUM(line_total_cents), 0)::BIGINT AS subtotal
            FROM sales_order_lines
            WHERE sales_order_id = $1 AND tenant_id = $2
        )
        UPDATE sales_orders SET
            subtotal_cents = (SELECT subtotal FROM line_sum),
            total_cents    = (SELECT subtotal FROM line_sum) + $3,
            updated_at     = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING subtotal_cents, total_cents
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(tax_cents)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

// ── Line writes ───────────────────────────────────────────────────────────────

pub async fn insert_line<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    sales_order_id: Uuid,
    item_id: Option<Uuid>,
    part_number: Option<&str>,
    description: &str,
    uom: &str,
    quantity: f64,
    unit_price_cents: i64,
    line_total_cents: i64,
    required_date: Option<NaiveDate>,
    promised_date: Option<NaiveDate>,
    warehouse_id: Option<Uuid>,
    notes: Option<&str>,
) -> Result<SalesOrderLine, sqlx::Error> {
    sqlx::query_as::<_, SalesOrderLine>(
        r#"
        INSERT INTO sales_order_lines (
            id, tenant_id, sales_order_id, line_number, item_id, part_number,
            description, uom, quantity, unit_price_cents, line_total_cents,
            required_date, promised_date, shipped_qty, warehouse_id, notes
        )
        SELECT $1, $2, $3,
               COALESCE((SELECT MAX(line_number) FROM sales_order_lines WHERE sales_order_id = $3), 0) + 1,
               $4, $5, $6, $7, $8, $9, $10, $11, $12, 0, $13, $14
        RETURNING id, tenant_id, sales_order_id, line_number, item_id, part_number,
                  description, uom, quantity, unit_price_cents, line_total_cents,
                  required_date, promised_date, shipped_qty, warehouse_id,
                  reservation_id, invoiced_at, notes
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(sales_order_id)
    .bind(item_id)
    .bind(part_number)
    .bind(description)
    .bind(uom)
    .bind(quantity)
    .bind(unit_price_cents)
    .bind(line_total_cents)
    .bind(required_date)
    .bind(promised_date)
    .bind(warehouse_id)
    .bind(notes)
    .fetch_one(executor)
    .await
}

pub async fn update_line<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    sales_order_id: Uuid,
    item_id: Option<Uuid>,
    part_number: Option<&str>,
    description: Option<&str>,
    uom: Option<&str>,
    quantity: f64,
    unit_price_cents: i64,
    line_total_cents: i64,
    required_date: Option<NaiveDate>,
    promised_date: Option<NaiveDate>,
    notes: Option<&str>,
) -> Result<SalesOrderLine, sqlx::Error> {
    sqlx::query_as::<_, SalesOrderLine>(
        r#"
        UPDATE sales_order_lines SET
            item_id          = COALESCE($4, item_id),
            part_number      = COALESCE($5, part_number),
            description      = COALESCE($6, description),
            uom              = COALESCE($7, uom),
            quantity         = $8,
            unit_price_cents = $9,
            line_total_cents = $10,
            required_date    = $11,
            promised_date    = $12,
            notes            = $13
        WHERE id = $1 AND tenant_id = $2 AND sales_order_id = $3
        RETURNING id, tenant_id, sales_order_id, line_number, item_id, part_number,
                  description, uom, quantity, unit_price_cents, line_total_cents,
                  required_date, promised_date, shipped_qty, warehouse_id,
                  reservation_id, invoiced_at, notes
        "#,
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(sales_order_id)
    .bind(item_id)
    .bind(part_number)
    .bind(description)
    .bind(uom)
    .bind(quantity)
    .bind(unit_price_cents)
    .bind(line_total_cents)
    .bind(required_date)
    .bind(promised_date)
    .bind(notes)
    .fetch_one(executor)
    .await
}

pub async fn delete_line<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    sales_order_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM sales_order_lines WHERE id = $1 AND tenant_id = $2 AND sales_order_id = $3",
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(sales_order_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn fetch_line<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    sales_order_id: Uuid,
) -> Result<Option<SalesOrderLine>, sqlx::Error> {
    sqlx::query_as::<_, SalesOrderLine>(
        r#"
        SELECT id, tenant_id, sales_order_id, line_number, item_id, part_number,
               description, uom, quantity, unit_price_cents, line_total_cents,
               required_date, promised_date, shipped_qty, warehouse_id,
               reservation_id, invoiced_at, notes
        FROM sales_order_lines
        WHERE id = $1 AND tenant_id = $2 AND sales_order_id = $3
        "#,
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(sales_order_id)
    .fetch_optional(executor)
    .await
}

pub async fn update_line_reservation<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    reservation_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sales_order_lines SET reservation_id = $3 WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(reservation_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update_line_shipped_qty<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
    shipped_qty: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sales_order_lines SET shipped_qty = shipped_qty + $3 WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_id)
    .bind(shipped_qty)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn mark_line_invoiced<'e>(
    executor: impl PgExecutor<'e>,
    line_id: Uuid,
    tenant_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE sales_order_lines SET invoiced_at = NOW() WHERE id = $1 AND tenant_id = $2",
    )
    .bind(line_id)
    .bind(tenant_id)
    .execute(executor)
    .await?;
    Ok(())
}
