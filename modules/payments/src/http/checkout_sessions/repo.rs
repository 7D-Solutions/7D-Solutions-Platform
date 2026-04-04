use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Row types (previously inline in handlers)
// ============================================================================

#[derive(sqlx::FromRow)]
pub struct ExistingSession {
    pub id: Uuid,
    pub processor_payment_id: String,
    pub client_secret: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct SessionDetailRow {
    pub status: String,
    pub processor_payment_id: String,
    pub invoice_id: String,
    pub tenant_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub return_url: Option<String>,
    pub cancel_url: Option<String>,
}

// ============================================================================
// Queries
// ============================================================================

pub async fn find_session_by_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idem_key: &str,
) -> Result<Option<ExistingSession>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, processor_payment_id, client_secret \
         FROM checkout_sessions \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await
}

pub async fn insert_checkout_session(
    pool: &PgPool,
    invoice_id: &str,
    tenant_id: &str,
    amount: i64,
    currency: &str,
    pi_id: &str,
    client_secret: &str,
    idem_key: &str,
    return_url: &Option<String>,
    cancel_url: &Option<String>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id,
             client_secret, idempotency_key, return_url, cancel_url)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(invoice_id)
    .bind(tenant_id)
    .bind(amount)
    .bind(currency)
    .bind(pi_id)
    .bind(client_secret)
    .bind(idem_key)
    .bind(return_url)
    .bind(cancel_url)
    .fetch_one(pool)
    .await
}

pub async fn find_session_details(
    pool: &PgPool,
    session_id: Uuid,
    tenant_id: &str,
) -> Result<Option<SessionDetailRow>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT status, processor_payment_id, invoice_id, tenant_id,
                  amount_minor, currency, return_url, cancel_url
           FROM checkout_sessions WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(session_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn update_session_status(
    pool: &PgPool,
    session_id: Uuid,
    status: &str,
) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("UPDATE checkout_sessions SET status = $1, updated_at = NOW() WHERE id = $2")
            .bind(status)
            .bind(session_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

pub async fn present_session(
    pool: &PgPool,
    session_id: Uuid,
    tenant_id: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created' AND tenant_id = $2",
    )
    .bind(session_id)
    .bind(tenant_id)
    .execute(pool)
    .await?
    .rows_affected())
}

pub async fn session_exists(
    pool: &PgPool,
    session_id: Uuid,
    tenant_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM checkout_sessions WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(session_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
}

pub async fn poll_session_status(
    pool: &PgPool,
    session_id: Uuid,
    tenant_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1 AND tenant_id = $2")
        .bind(session_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn update_status_by_processor_id(
    pool: &PgPool,
    processor_payment_id: &str,
    new_status: &str,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "UPDATE checkout_sessions \
         SET status = $1, updated_at = NOW() \
         WHERE processor_payment_id = $2 \
         AND status IN ('created', 'presented')",
    )
    .bind(new_status)
    .bind(processor_payment_id)
    .execute(pool)
    .await?
    .rows_affected())
}
