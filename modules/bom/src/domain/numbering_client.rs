use platform_client_numbering as numbering_typed;
use platform_sdk::{PlatformClient, VerifiedClaims};
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
    /// Production: calls the Numbering service over HTTP via typed client.
    Http {
        base_url: String,
        client: reqwest::Client,
    },
    /// Test / direct: allocates directly against the Numbering database.
    Direct { pool: PgPool },
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
        claims: &VerifiedClaims,
    ) -> Result<String, BomError> {
        match &self.mode {
            Mode::Http {
                base_url, client, ..
            } => {
                let token = extract_bearer_token(auth_header)?;
                let platform = PlatformClient::new(base_url.clone())
                    .with_bearer_token(token.to_string());
                let typed = numbering_typed::NumberingClient::new(platform);
                let body = numbering_typed::AllocateRequest {
                    entity: "ECO".into(),
                    idempotency_key: idempotency_key.into(),
                    gap_free: None,
                };
                let alloc = typed.allocate(claims, &body).await.map_err(|e| {
                    GuardError::Validation(format!("Numbering service error: {e}"))
                })?;
                Ok(alloc
                    .formatted_number
                    .unwrap_or_else(|| format!("ECO-{:05}", alloc.number_value)))
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
        claims: &VerifiedClaims,
    ) {
        if let Mode::Http {
            base_url, client, ..
        } = &self.mode
        {
            let Some(token) = auth_header
                .and_then(|h| h.strip_prefix("Bearer ").or(Some(h)))
            else {
                return;
            };
            let platform = PlatformClient::new(base_url.clone())
                .with_bearer_token(token.to_string());
            let typed = numbering_typed::NumberingClient::new(platform);
            let body = numbering_typed::ConfirmRequest {
                entity: "ECO".into(),
                idempotency_key: idempotency_key.into(),
            };
            if let Err(e) = typed.confirm(claims, &body).await {
                tracing::warn!("Numbering confirm call failed (non-fatal): {e}");
            }
        }
        // Direct mode uses standard allocation — already confirmed.
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the bare token from an `Authorization: Bearer <token>` header.
fn extract_bearer_token(auth_header: Option<&str>) -> Result<&str, BomError> {
    let header = auth_header.ok_or_else(|| {
        GuardError::Validation(
            "Authorization header required for numbering service".to_string(),
        )
    })?;
    Ok(header.strip_prefix("Bearer ").unwrap_or(header))
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
