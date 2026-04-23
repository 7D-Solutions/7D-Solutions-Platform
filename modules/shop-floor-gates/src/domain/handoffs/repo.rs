use sqlx::PgPool;
use uuid::Uuid;

use super::{ListHandoffsQuery, OperationHandoff};

pub async fn insert_handoff(pool: &PgPool, h: &OperationHandoff) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO operation_handoffs
           (id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
            initiation_type, status, quantity, unit_of_measure, lot_number, serial_numbers, notes,
            initiated_by, initiated_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::float8::numeric,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(h.id)
    .bind(&h.tenant_id)
    .bind(&h.handoff_number)
    .bind(h.work_order_id)
    .bind(h.source_operation_id)
    .bind(h.dest_operation_id)
    .bind(&h.initiation_type)
    .bind(&h.status)
    .bind(h.quantity)
    .bind(&h.unit_of_measure)
    .bind(&h.lot_number)
    .bind(&h.serial_numbers)
    .bind(&h.notes)
    .bind(h.initiated_by)
    .bind(h.initiated_at)
    .bind(h.created_at)
    .bind(h.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_handoff(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<OperationHandoff>, sqlx::Error> {
    let sql = r#"SELECT id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
        initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
        initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
        cancelled_by, cancelled_at, cancel_reason, created_at, updated_at
        FROM operation_handoffs WHERE id = $1 AND tenant_id = $2"#;
    sqlx::query_as::<_, OperationHandoff>(sql)
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_handoffs(
    pool: &PgPool,
    tenant_id: &str,
    q: &ListHandoffsQuery,
) -> Result<Vec<OperationHandoff>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let sql = r#"SELECT id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
        initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
        initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
        cancelled_by, cancelled_at, cancel_reason, created_at, updated_at
        FROM operation_handoffs
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR status = $2)
          AND ($3::uuid IS NULL OR work_order_id = $3)
          AND ($4::uuid IS NULL OR source_operation_id = $4)
          AND ($5::uuid IS NULL OR dest_operation_id = $5)
        ORDER BY initiated_at DESC
        LIMIT $6 OFFSET $7"#;
    sqlx::query_as::<_, OperationHandoff>(sql)
        .bind(tenant_id)
        .bind(&q.status)
        .bind(q.work_order_id)
        .bind(q.source_operation_id)
        .bind(q.dest_operation_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

pub async fn accept_handoff(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    accepted_by: Uuid,
) -> Result<Option<OperationHandoff>, sqlx::Error> {
    let sql = r#"UPDATE operation_handoffs
        SET status = 'accepted', accepted_by = $3, accepted_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
        RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
            initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
            initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
            cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#;
    sqlx::query_as::<_, OperationHandoff>(sql)
        .bind(id)
        .bind(tenant_id)
        .bind(accepted_by)
        .fetch_optional(pool)
        .await
}

pub async fn reject_handoff(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    rejected_by: Uuid,
    rejection_reason: Option<&str>,
) -> Result<Option<OperationHandoff>, sqlx::Error> {
    let sql = r#"UPDATE operation_handoffs
        SET status = 'rejected', rejected_by = $3, rejected_at = NOW(),
            rejection_reason = $4, updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
        RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
            initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
            initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
            cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#;
    sqlx::query_as::<_, OperationHandoff>(sql)
        .bind(id)
        .bind(tenant_id)
        .bind(rejected_by)
        .bind(rejection_reason)
        .fetch_optional(pool)
        .await
}

pub async fn cancel_handoff(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    cancelled_by: Uuid,
    cancel_reason: Option<&str>,
) -> Result<Option<OperationHandoff>, sqlx::Error> {
    let sql = r#"UPDATE operation_handoffs
        SET status = 'cancelled', cancelled_by = $3, cancelled_at = NOW(),
            cancel_reason = $4, updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
        RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
            initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
            initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
            cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#;
    sqlx::query_as::<_, OperationHandoff>(sql)
        .bind(id)
        .bind(tenant_id)
        .bind(cancelled_by)
        .bind(cancel_reason)
        .fetch_optional(pool)
        .await
}
