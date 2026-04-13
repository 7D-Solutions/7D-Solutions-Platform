use platform_client_numbering as numbering_typed;
use platform_sdk::{PlatformClient, VerifiedClaims};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::work_orders::WorkOrderError;

// ============================================================================
// Public types
// ============================================================================

pub struct NumberingClient {
    mode: Mode,
}

enum Mode {
    /// Production: calls the Numbering service via SDK-wired platform client.
    Platform { client: PlatformClient },
    /// Test / direct: allocates directly against the Numbering database.
    Direct { pool: PgPool },
}

// ============================================================================
// Construction
// ============================================================================

impl platform_sdk::PlatformService for NumberingClient {
    const SERVICE_NAME: &'static str = "numbering";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self {
            mode: Mode::Platform { client },
        }
    }
}

impl NumberingClient {
    /// Build a client that allocates directly against the Numbering database.
    /// Used in integration tests where no HTTP server is running.
    pub fn direct(pool: PgPool) -> Self {
        Self {
            mode: Mode::Direct { pool },
        }
    }

    // -----------------------------------------------------------------------
    // Allocate
    // -----------------------------------------------------------------------

    /// Allocate the next WO number for a tenant.
    ///
    /// Returns a formatted string like `"WO-00001"`.
    /// `claims` are required for Platform mode (used in JWT forwarding).
    pub async fn allocate_wo_number(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
        claims: &VerifiedClaims,
    ) -> Result<String, WorkOrderError> {
        match &self.mode {
            Mode::Platform { client } => {
                let typed = numbering_typed::NumberingClient::new(client.clone());
                let body = numbering_typed::AllocateRequest {
                    entity: "WO".into(),
                    idempotency_key: idempotency_key.into(),
                    gap_free: None,
                };
                let alloc = typed.allocate(claims, &body).await.map_err(|e| {
                    WorkOrderError::NumberingService(format!("Numbering service error: {e}"))
                })?;
                Ok(alloc
                    .formatted_number
                    .unwrap_or_else(|| format!("WO-{:05}", alloc.number_value)))
            }
            Mode::Direct { pool } => {
                allocate_direct(pool, tenant_id, idempotency_key).await
            }
        }
    }
    // -----------------------------------------------------------------------
    // Void
    // -----------------------------------------------------------------------

    /// Best-effort compensating action: void (un-allocate) the WO number
    /// associated with `idempotency_key` so that it can be reclaimed on
    /// subsequent sequential allocations.
    ///
    /// This is called in the error path of `composite_create` when the
    /// Postgres INSERT fails for a reason other than a duplicate-number
    /// constraint (e.g. a transient DB error).  Failure here is non-fatal —
    /// the idempotency_key still ensures the SAME number is returned on retry.
    ///
    /// In Platform mode: no-op (the Numbering service does not yet expose a
    /// void endpoint).  A warning is logged; the allocated number remains
    /// reserved but is returned on re-submission with the same idempotency_key.
    ///
    /// In Direct mode: deletes the `issued_numbers` row so the number is
    /// immediately available for reuse.
    pub async fn void_wo_number(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> Result<(), WorkOrderError> {
        match &self.mode {
            Mode::Platform { .. } => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    idempotency_key = %idempotency_key,
                    "void_wo_number: Numbering service has no void endpoint — \
                     allocated number remains reserved; retry with same idempotency_key \
                     will return the same number"
                );
                Ok(())
            }
            Mode::Direct { pool } => void_direct(pool, tenant_id, idempotency_key).await,
        }
    }
}

// ============================================================================
// Direct-DB implementation (mirrors numbering service SQL)
// ============================================================================

async fn allocate_direct(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<String, WorkOrderError> {
    let tenant_uuid: Uuid = tenant_id.parse().map_err(|_| {
        WorkOrderError::Validation(format!(
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
    .await
    .map_err(|e| WorkOrderError::Database(e))?;

    if let Some((val,)) = existing {
        return Ok(format!("WO-{:05}", val));
    }

    let mut tx = pool.begin().await.map_err(|e| WorkOrderError::Database(e))?;

    let (next_value,): (i64,) = sqlx::query_as(
        "INSERT INTO sequences (tenant_id, entity, current_value) \
         VALUES ($1, $2, 1) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET current_value = sequences.current_value + 1, \
                       updated_at = NOW() \
         RETURNING current_value",
    )
    .bind(tenant_uuid)
    .bind("WO")
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| WorkOrderError::Database(e))?;

    sqlx::query(
        "INSERT INTO issued_numbers \
         (tenant_id, entity, number_value, idempotency_key) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_uuid)
    .bind("WO")
    .bind(next_value)
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await
    .map_err(|e| WorkOrderError::Database(e))?;

    tx.commit().await.map_err(|e| WorkOrderError::Database(e))?;

    Ok(format!("WO-{:05}", next_value))
}

async fn void_direct(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<(), WorkOrderError> {
    let tenant_uuid: Uuid = tenant_id.parse().map_err(|_| {
        WorkOrderError::Validation(format!(
            "Invalid tenant_id for numbering void: {}",
            tenant_id
        ))
    })?;

    sqlx::query(
        "DELETE FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_uuid)
    .bind(idempotency_key)
    .execute(pool)
    .await
    .map_err(WorkOrderError::Database)?;

    Ok(())
}
