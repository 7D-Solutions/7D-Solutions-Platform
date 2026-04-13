use platform_sdk::{PlatformClient, VerifiedClaims};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::work_orders::WorkOrderError;

// ============================================================================
// Public types
// ============================================================================

pub struct BomRevisionClient {
    mode: Mode,
}

enum Mode {
    /// Production: calls the BOM service via SDK-wired platform client.
    Platform { client: PlatformClient },
    /// Test / direct: validates directly against the BOM database.
    Direct { pool: PgPool },
    /// Test utility: skips all validation and always returns Ok(()).
    /// Use this in tests that exercise other aspects of composite_create
    /// and do not need BOM revision validation.
    Permissive,
}

// ============================================================================
// Construction
// ============================================================================

impl platform_sdk::PlatformService for BomRevisionClient {
    const SERVICE_NAME: &'static str = "bom";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self {
            mode: Mode::Platform { client },
        }
    }
}

impl BomRevisionClient {
    /// Build a client that validates directly against the BOM database.
    /// Used in integration tests where no HTTP server is running.
    pub fn direct(pool: PgPool) -> Self {
        Self {
            mode: Mode::Direct { pool },
        }
    }

    /// Build a no-op client that skips BOM validation.
    /// Use in tests that are not exercising BOM validation logic.
    pub fn permissive() -> Self {
        Self {
            mode: Mode::Permissive,
        }
    }

    // -----------------------------------------------------------------------
    // Validate
    // -----------------------------------------------------------------------

    /// Validate that a BOM revision exists and is in 'effective' status.
    ///
    /// Returns `Ok(())` when the revision is found and effective.
    /// Returns `Err(WorkOrderError::Validation(...))` when:
    ///   - The revision does not exist in the tenant's BOM database, or
    ///   - The revision exists but is not in 'effective' status
    ///     (e.g. still 'draft' or 'superseded').
    ///
    /// This check is performed BEFORE any Postgres transaction is opened so
    /// that a slow or failed BOM lookup never holds an open TX.
    pub async fn validate_revision(
        &self,
        tenant_id: &str,
        revision_id: Uuid,
        claims: &VerifiedClaims,
    ) -> Result<(), WorkOrderError> {
        match &self.mode {
            Mode::Platform { client } => {
                validate_revision_platform(client, tenant_id, revision_id, claims).await
            }
            Mode::Direct { pool } => {
                validate_revision_direct(pool, tenant_id, revision_id).await
            }
            Mode::Permissive => Ok(()),
        }
    }
}

// ============================================================================
// Platform-mode implementation (HTTP → BOM service)
// ============================================================================

async fn validate_revision_platform(
    client: &PlatformClient,
    _tenant_id: &str,
    revision_id: Uuid,
    claims: &VerifiedClaims,
) -> Result<(), WorkOrderError> {
    let path = format!("/api/bom/revisions/{}", revision_id);
    let resp = client.get(&path, claims).await.map_err(|e| {
        WorkOrderError::Validation(format!("BOM service unreachable: {}", e))
    })?;

    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await.map_err(|e| {
                WorkOrderError::Validation(format!("BOM service response parse error: {}", e))
            })?;
            let status = body["status"].as_str().unwrap_or("unknown");
            if status != "effective" {
                return Err(WorkOrderError::Validation(format!(
                    "BOM revision {} has status '{}' — only 'effective' revisions may be used on a work order",
                    revision_id, status
                )));
            }
            Ok(())
        }
        404 => Err(WorkOrderError::Validation(format!(
            "BOM revision {} not found",
            revision_id
        ))),
        code => Err(WorkOrderError::Validation(format!(
            "BOM service returned unexpected status {} for revision {}",
            code, revision_id
        ))),
    }
}

// ============================================================================
// Direct-DB implementation (mirrors BOM service SQL)
// ============================================================================

async fn validate_revision_direct(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
) -> Result<(), WorkOrderError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM bom_revisions WHERE id = $1 AND tenant_id = $2",
    )
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
    .map_err(WorkOrderError::Database)?;

    match row {
        None => Err(WorkOrderError::Validation(format!(
            "BOM revision {} not found",
            revision_id
        ))),
        Some((status,)) if status != "effective" => Err(WorkOrderError::Validation(format!(
            "BOM revision {} has status '{}' — only 'effective' revisions may be used on a work order",
            revision_id, status
        ))),
        Some(_) => Ok(()),
    }
}
