use sqlx::PgPool;
use uuid::Uuid;

use super::{ListVerificationsQuery, OperationStartVerification};

pub async fn insert_verification(
    pool: &PgPool,
    v: &OperationStartVerification,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO operation_start_verifications
           (id, tenant_id, work_order_id, operation_id, status, drawing_verified,
            material_verified, instruction_verified, operator_id, notes, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(v.id)
    .bind(&v.tenant_id)
    .bind(v.work_order_id)
    .bind(v.operation_id)
    .bind(&v.status)
    .bind(v.drawing_verified)
    .bind(v.material_verified)
    .bind(v.instruction_verified)
    .bind(v.operator_id)
    .bind(&v.notes)
    .bind(v.created_at)
    .bind(v.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_verification(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<OperationStartVerification>, sqlx::Error> {
    let sql = "SELECT * FROM operation_start_verifications WHERE id = $1 AND tenant_id = $2";
    sqlx::query_as::<_, OperationStartVerification>(sql)
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn fetch_verification_for_operation(
    pool: &PgPool,
    work_order_id: Uuid,
    operation_id: Uuid,
    tenant_id: &str,
) -> Result<Option<OperationStartVerification>, sqlx::Error> {
    let sql = "SELECT * FROM operation_start_verifications WHERE work_order_id = $1 AND operation_id = $2 AND tenant_id = $3";
    sqlx::query_as::<_, OperationStartVerification>(sql)
        .bind(work_order_id)
        .bind(operation_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_verifications(
    pool: &PgPool,
    tenant_id: &str,
    q: &ListVerificationsQuery,
) -> Result<Vec<OperationStartVerification>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let sql = r#"SELECT * FROM operation_start_verifications
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR status = $2)
          AND ($3::uuid IS NULL OR work_order_id = $3)
          AND ($4::uuid IS NULL OR operation_id = $4)
        ORDER BY created_at DESC
        LIMIT $5 OFFSET $6"#;
    sqlx::query_as::<_, OperationStartVerification>(sql)
        .bind(tenant_id)
        .bind(&q.status)
        .bind(q.work_order_id)
        .bind(q.operation_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}
