//! Dispute repository — all SQL operations for the disputes domain.

use sqlx::PgExecutor;

use crate::models::Dispute;

/// Fetch a dispute by ID with tenant isolation.
pub async fn fetch_by_id<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Dispute>, sqlx::Error> {
    sqlx::query_as::<_, Dispute>(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Count disputes matching filters.
pub async fn count_disputes<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    charge_id: Option<i32>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from("SELECT COUNT(*) FROM ar_disputes WHERE app_id = $1");
    let mut idx = 2;
    if charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND status = ${idx}"));
    }
    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(cid) = charge_id {
        q = q.bind(cid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// List disputes with optional filters and pagination.
pub async fn list_disputes<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    charge_id: Option<i32>,
    status: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Dispute>, sqlx::Error> {
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE app_id = $1
        "#,
    );

    let mut idx = 2;
    if charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${idx}"));
        idx += 1;
    }
    if status.is_some() {
        sql.push_str(&format!(" AND status = ${idx}"));
        idx += 1;
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${idx} OFFSET ${}",
        idx + 1
    ));

    let mut q = sqlx::query_as::<_, Dispute>(&sql).bind(app_id);
    if let Some(cid) = charge_id {
        q = q.bind(cid);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.bind(limit).bind(offset).fetch_all(executor).await
}

/// Update dispute status to under_review after evidence submission.
pub async fn set_under_review<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<Dispute, sqlx::Error> {
    sqlx::query_as::<_, Dispute>(
        r#"
        UPDATE ar_disputes
        SET status = 'under_review', updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .fetch_one(executor)
    .await
}
