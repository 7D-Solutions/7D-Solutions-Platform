use sqlx::PgPool;
use uuid::Uuid;

use super::StatusLabel;

pub async fn upsert_label(
    pool: &PgPool,
    table: &str,
    _tenant_id: &str,
    label: &StatusLabel,
) -> Result<StatusLabel, sqlx::Error> {
    let sql = format!(
        r#"INSERT INTO {table} (id, tenant_id, status_key, display_name, color_hex, sort_order)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (tenant_id, status_key) DO UPDATE
             SET display_name = EXCLUDED.display_name,
                 color_hex = EXCLUDED.color_hex,
                 sort_order = EXCLUDED.sort_order
           RETURNING *"#
    );
    sqlx::query_as::<_, StatusLabel>(&sql)
        .bind(label.id)
        .bind(&label.tenant_id)
        .bind(&label.status_key)
        .bind(&label.display_name)
        .bind(&label.color_hex)
        .bind(label.sort_order)
        .fetch_one(pool)
        .await
}

pub async fn list_labels(
    pool: &PgPool,
    table: &str,
    tenant_id: &str,
) -> Result<Vec<StatusLabel>, sqlx::Error> {
    let sql = format!(
        "SELECT * FROM {table} WHERE tenant_id = $1 ORDER BY sort_order ASC, status_key ASC"
    );
    sqlx::query_as::<_, StatusLabel>(&sql)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
}

pub async fn delete_label(
    pool: &PgPool,
    table: &str,
    id: Uuid,
    tenant_id: &str,
) -> Result<bool, sqlx::Error> {
    let sql = format!("DELETE FROM {table} WHERE id = $1 AND tenant_id = $2");
    let result = sqlx::query(&sql)
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
