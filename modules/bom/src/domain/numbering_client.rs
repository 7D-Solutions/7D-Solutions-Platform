use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::bom_service::BomError;
use crate::domain::guards::GuardError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

pub struct NumberingClient {
    mode: Mode,
}

enum Mode {
    /// Production: calls the Numbering service over HTTP.
    Http {
        base_url: String,
        client: reqwest::Client,
    },
    /// Test / direct: allocates directly against the Numbering database.
    Direct { pool: PgPool },
}

// ---------------------------------------------------------------------------
// HTTP request / response shapes (match numbering service API)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AllocateRequest {
    entity: String,
    idempotency_key: String,
}

#[derive(Deserialize)]
struct AllocateResponse {
    number_value: i64,
    formatted_number: Option<String>,
}

#[derive(Serialize)]
struct ConfirmRequest {
    entity: String,
    idempotency_key: String,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

impl NumberingClient {
    /// Build a client that calls the Numbering HTTP service.
    pub fn http(base_url: String) -> Self {
        Self {
            mode: Mode::Http {
                base_url,
                client: reqwest::Client::new(),
            },
        }
    }

    /// Build a client that allocates directly against the Numbering database
    /// (used in integration tests where no HTTP server is running).
    pub fn direct(pool: PgPool) -> Self {
        Self {
            mode: Mode::Direct { pool },
        }
    }

    // -----------------------------------------------------------------------
    // Allocate
    // -----------------------------------------------------------------------

    /// Allocate the next ECO number for a tenant.
    ///
    /// Returns a formatted string like `"ECO-00001"`.
    /// `auth_header` is required for HTTP mode (forwarded JWT).
    pub async fn allocate_eco_number(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
        auth_header: Option<&str>,
    ) -> Result<String, BomError> {
        match &self.mode {
            Mode::Http {
                base_url, client, ..
            } => {
                allocate_http(client, base_url, auth_header, idempotency_key).await
            }
            Mode::Direct { pool } => {
                allocate_direct(pool, tenant_id, idempotency_key).await
            }
        }
    }

    /// Best-effort confirm after successful ECO insert (only meaningful in
    /// gap-free mode; harmless for standard allocations).
    pub async fn confirm_eco_number(
        &self,
        idempotency_key: &str,
        auth_header: Option<&str>,
    ) {
        if let Mode::Http {
            base_url, client, ..
        } = &self.mode
        {
            confirm_http(client, base_url, auth_header, idempotency_key).await;
        }
        // Direct mode uses standard allocation — already confirmed.
    }
}

// ---------------------------------------------------------------------------
// HTTP implementation
// ---------------------------------------------------------------------------

async fn allocate_http(
    client: &reqwest::Client,
    base_url: &str,
    auth_header: Option<&str>,
    idempotency_key: &str,
) -> Result<String, BomError> {
    let auth = auth_header.ok_or_else(|| {
        GuardError::Validation(
            "Authorization header required for numbering service".to_string(),
        )
    })?;

    let url = format!("{}/allocate", base_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .header("Authorization", auth)
        .json(&AllocateRequest {
            entity: "ECO".into(),
            idempotency_key: idempotency_key.into(),
        })
        .send()
        .await
        .map_err(|e| {
            GuardError::Validation(format!("Numbering service unreachable: {}", e))
        })?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(GuardError::Validation(format!(
            "Numbering service returned {}: {}",
            status, body
        ))
        .into());
    }

    let alloc: AllocateResponse = resp.json().await.map_err(|e| {
        GuardError::Validation(format!("Numbering response parse error: {}", e))
    })?;

    Ok(alloc
        .formatted_number
        .unwrap_or_else(|| format!("ECO-{:05}", alloc.number_value)))
}

async fn confirm_http(
    client: &reqwest::Client,
    base_url: &str,
    auth_header: Option<&str>,
    idempotency_key: &str,
) {
    let Some(auth) = auth_header else { return };
    let url = format!("{}/confirm", base_url.trim_end_matches('/'));

    let result = client
        .post(&url)
        .header("Authorization", auth)
        .json(&ConfirmRequest {
            entity: "ECO".into(),
            idempotency_key: idempotency_key.into(),
        })
        .send()
        .await;

    if let Err(e) = result {
        tracing::warn!("Numbering confirm call failed (non-fatal): {}", e);
    }
}

// ---------------------------------------------------------------------------
// Direct-DB implementation (mirrors numbering service SQL)
// ---------------------------------------------------------------------------

async fn allocate_direct(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<String, BomError> {
    let tenant_uuid: Uuid = tenant_id.parse().map_err(|_| {
        GuardError::Validation(format!(
            "Invalid tenant_id for numbering: {}",
            tenant_id
        ))
    })?;

    // Idempotency check
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT number_value FROM issued_numbers \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_uuid)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await?;

    if let Some((val,)) = existing {
        return Ok(format!("ECO-{:05}", val));
    }

    let mut tx = pool.begin().await?;

    let (next_value,): (i64,) = sqlx::query_as(
        "INSERT INTO sequences (tenant_id, entity, current_value) \
         VALUES ($1, $2, 1) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET current_value = sequences.current_value + 1, \
                       updated_at = NOW() \
         RETURNING current_value",
    )
    .bind(tenant_uuid)
    .bind("ECO")
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO issued_numbers \
         (tenant_id, entity, number_value, idempotency_key) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_uuid)
    .bind("ECO")
    .bind(next_value)
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(format!("ECO-{:05}", next_value))
}
