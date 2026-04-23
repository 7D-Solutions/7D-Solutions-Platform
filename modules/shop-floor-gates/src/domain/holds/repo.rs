use sqlx::PgPool;
use uuid::Uuid;

use super::{ListHoldsQuery, TravelerHold};

pub async fn insert_hold(pool: &PgPool, hold: &TravelerHold) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO traveler_holds
           (id, tenant_id, hold_number, hold_type, scope, work_order_id, operation_id,
            reason, status, release_authority, placed_by, placed_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(hold.id)
    .bind(&hold.tenant_id)
    .bind(&hold.hold_number)
    .bind(&hold.hold_type)
    .bind(&hold.scope)
    .bind(hold.work_order_id)
    .bind(hold.operation_id)
    .bind(&hold.reason)
    .bind(&hold.status)
    .bind(&hold.release_authority)
    .bind(hold.placed_by)
    .bind(hold.placed_at)
    .bind(hold.created_at)
    .bind(hold.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_hold(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<TravelerHold>, sqlx::Error> {
    let sql = "SELECT * FROM traveler_holds WHERE id = $1 AND tenant_id = $2";
    sqlx::query_as::<_, TravelerHold>(sql)
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn fetch_hold_by_number(
    pool: &PgPool,
    hold_number: &str,
    tenant_id: &str,
) -> Result<Option<TravelerHold>, sqlx::Error> {
    let sql = "SELECT * FROM traveler_holds WHERE hold_number = $1 AND tenant_id = $2";
    sqlx::query_as::<_, TravelerHold>(sql)
        .bind(hold_number)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_holds(
    pool: &PgPool,
    tenant_id: &str,
    q: &ListHoldsQuery,
) -> Result<Vec<TravelerHold>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let sql = r#"SELECT * FROM traveler_holds
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR status = $2)
          AND ($3::text IS NULL OR hold_type = $3)
          AND ($4::uuid IS NULL OR work_order_id = $4)
          AND ($5::uuid IS NULL OR operation_id = $5)
        ORDER BY placed_at DESC
        LIMIT $6 OFFSET $7"#;

    sqlx::query_as::<_, TravelerHold>(sql)
        .bind(tenant_id)
        .bind(&q.status)
        .bind(&q.hold_type)
        .bind(q.work_order_id)
        .bind(q.operation_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

pub async fn release_hold(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    released_by: Uuid,
    release_notes: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE traveler_holds
           SET status = 'released', released_by = $3, released_at = NOW(),
               release_notes = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'active'"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(released_by)
    .bind(release_notes)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn cancel_hold(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    cancelled_by: Uuid,
    cancel_reason: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE traveler_holds
           SET status = 'cancelled', cancelled_by = $3, cancelled_at = NOW(),
               cancel_reason = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'active'"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(cancelled_by)
    .bind(cancel_reason)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn release_all_active_for_work_order(
    pool: &PgPool,
    work_order_id: Uuid,
    tenant_id: &str,
    released_by: Uuid,
    release_notes: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"UPDATE traveler_holds
           SET status = 'released', released_by = $3, released_at = NOW(),
               release_notes = $4, updated_at = NOW()
           WHERE work_order_id = $1 AND tenant_id = $2 AND status = 'active'"#,
    )
    .bind(work_order_id)
    .bind(tenant_id)
    .bind(released_by)
    .bind(release_notes)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn count_active_holds_for_work_order(
    pool: &PgPool,
    work_order_id: Uuid,
    tenant_id: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM traveler_holds WHERE work_order_id = $1 AND tenant_id = $2 AND status = 'active'",
    )
    .bind(work_order_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}
