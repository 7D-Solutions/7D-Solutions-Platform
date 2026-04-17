//! Status label repository.

use sqlx::PgExecutor;
use uuid::Uuid;

use super::StatusLabel;

pub async fn list_labels<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    label_type: Option<&str>,
) -> Result<Vec<StatusLabel>, sqlx::Error> {
    sqlx::query_as::<_, StatusLabel>(
        r#"
        SELECT id, tenant_id, label_type, status_key, display_name, color_hex, sort_order
        FROM sales_order_status_labels
        WHERE tenant_id = $1 AND ($2::text IS NULL OR label_type = $2)
        ORDER BY label_type, sort_order, status_key
        "#,
    )
    .bind(tenant_id)
    .bind(label_type)
    .fetch_all(executor)
    .await
}

pub async fn upsert_label<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    tenant_id: &str,
    label_type: &str,
    status_key: &str,
    display_name: &str,
    color_hex: Option<&str>,
    sort_order: i32,
) -> Result<StatusLabel, sqlx::Error> {
    sqlx::query_as::<_, StatusLabel>(
        r#"
        INSERT INTO sales_order_status_labels
            (id, tenant_id, label_type, status_key, display_name, color_hex, sort_order)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (tenant_id, label_type, status_key) DO UPDATE SET
            display_name = EXCLUDED.display_name,
            color_hex    = EXCLUDED.color_hex,
            sort_order   = EXCLUDED.sort_order
        RETURNING id, tenant_id, label_type, status_key, display_name, color_hex, sort_order
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(label_type)
    .bind(status_key)
    .bind(display_name)
    .bind(color_hex)
    .bind(sort_order)
    .fetch_one(executor)
    .await
}

pub async fn delete_label<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    label_type: &str,
    status_key: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM sales_order_status_labels WHERE tenant_id = $1 AND label_type = $2 AND status_key = $3",
    )
    .bind(tenant_id)
    .bind(label_type)
    .bind(status_key)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}
