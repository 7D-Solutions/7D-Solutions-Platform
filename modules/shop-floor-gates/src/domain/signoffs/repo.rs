use sqlx::PgPool;
use uuid::Uuid;

use super::{ListSignoffsQuery, Signoff};

pub async fn insert_signoff(pool: &PgPool, s: &Signoff) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO signoffs
           (id, tenant_id, entity_type, entity_id, role, signoff_number, signed_by, signed_at, signature_text, notes, created_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(s.id)
    .bind(&s.tenant_id)
    .bind(&s.entity_type)
    .bind(s.entity_id)
    .bind(&s.role)
    .bind(&s.signoff_number)
    .bind(s.signed_by)
    .bind(s.signed_at)
    .bind(&s.signature_text)
    .bind(&s.notes)
    .bind(s.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_signoff(pool: &PgPool, id: Uuid, tenant_id: &str) -> Result<Option<Signoff>, sqlx::Error> {
    let sql = "SELECT * FROM signoffs WHERE id = $1 AND tenant_id = $2";
    sqlx::query_as::<_, Signoff>(sql)
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_signoffs(pool: &PgPool, tenant_id: &str, q: &ListSignoffsQuery) -> Result<Vec<Signoff>, sqlx::Error> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let sql = r#"SELECT * FROM signoffs
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR entity_type = $2)
          AND ($3::uuid IS NULL OR entity_id = $3)
          AND ($4::text IS NULL OR role = $4)
        ORDER BY signed_at DESC
        LIMIT $5 OFFSET $6"#;
    sqlx::query_as::<_, Signoff>(sql)
        .bind(tenant_id)
        .bind(&q.entity_type)
        .bind(q.entity_id)
        .bind(&q.role)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}
