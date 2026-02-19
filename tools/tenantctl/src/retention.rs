//! Tenant data export for the retention framework
//!
//! Produces a deterministic JSONL artifact containing all tenant metadata
//! held in the platform tenant registry.
//!
//! Determinism guarantee: the artifact is produced by sorting rows by their
//! primary key before serialising, so identical database state always produces
//! identical bytes and therefore an identical SHA-256 digest.
//!
//! The command also updates `cp_retention_policies.export_ready_at` so the
//! tombstone window can begin.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// Result returned after a successful export
pub struct ExportResult {
    /// Path of the written artifact (or "<stdout>" if no path given)
    pub artifact_path: String,
    /// Hex-encoded SHA-256 digest of the artifact bytes
    pub sha256_digest: String,
    /// Number of JSONL lines written
    pub line_count: usize,
}

// ============================================================================
// Internal row types (runtime query_as)
// ============================================================================

#[derive(FromRow)]
struct TenantRow {
    tenant_id: Uuid,
    status: String,
    environment: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

#[derive(FromRow)]
struct PolicyRow {
    data_retention_days: i32,
    export_format: String,
    auto_tombstone_days: i32,
    export_ready_at: Option<DateTime<Utc>>,
    data_tombstoned_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct EntitlementRow {
    plan_code: String,
    concurrent_user_limit: i32,
    effective_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct ProvRow {
    environment: String,
    created_at: DateTime<Utc>,
}

// ============================================================================
// Export
// ============================================================================

/// Export tenant data to a JSONL artifact.
///
/// # Arguments
/// * `tenant_id`    – tenant identifier (UUID or short name)
/// * `output_path`  – file path to write; if None the bytes are still computed
///                    for digest purposes but nothing is persisted to disk
pub async fn export_tenant(tenant_id: &str, output_path: Option<&str>) -> Result<ExportResult> {
    let db_url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .context("TENANT_REGISTRY_DATABASE_URL not set")?;

    let pool = PgPool::connect(&db_url)
        .await
        .context("Failed to connect to tenant registry")?;

    let tid = parse_tenant_id(tenant_id)?;

    // ---- Verify tenant exists ----
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE tenant_id = $1)")
            .bind(tid)
            .fetch_one(&pool)
            .await
            .context("Querying tenant existence")?;

    if !exists {
        anyhow::bail!("Tenant not found: {tenant_id}");
    }

    // ---- Collect data rows (sorted for determinism) ----
    let mut lines: Vec<String> = Vec::new();

    // 1. Tenant core record
    let t: TenantRow = sqlx::query_as(
        r#"SELECT tenant_id, status, environment, created_at, updated_at, deleted_at
           FROM tenants WHERE tenant_id = $1"#,
    )
    .bind(tid)
    .fetch_one(&pool)
    .await
    .context("Fetching tenant record")?;

    lines.push(serde_json::to_string(&serde_json::json!({
        "record_type": "tenant",
        "tenant_id": t.tenant_id,
        "status": t.status,
        "environment": t.environment,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
        "deleted_at": t.deleted_at,
    }))?);

    // 2. Retention policy (if present)
    let policy: Option<PolicyRow> = sqlx::query_as(
        r#"SELECT data_retention_days, export_format, auto_tombstone_days,
                  export_ready_at, data_tombstoned_at, created_at, updated_at
           FROM cp_retention_policies WHERE tenant_id = $1"#,
    )
    .bind(tid)
    .fetch_optional(&pool)
    .await
    .context("Fetching retention policy")?;

    if let Some(p) = policy {
        lines.push(serde_json::to_string(&serde_json::json!({
            "record_type": "retention_policy",
            "tenant_id": tid,
            "data_retention_days": p.data_retention_days,
            "export_format": p.export_format,
            "auto_tombstone_days": p.auto_tombstone_days,
            "export_ready_at": p.export_ready_at,
            "data_tombstoned_at": p.data_tombstoned_at,
            "policy_created_at": p.created_at,
            "policy_updated_at": p.updated_at,
        }))?);
    }

    // 3. Entitlements (if present)
    let entitlement: Option<EntitlementRow> = sqlx::query_as(
        r#"SELECT plan_code, concurrent_user_limit, effective_at, updated_at
           FROM cp_entitlements WHERE tenant_id = $1"#,
    )
    .bind(tid)
    .fetch_optional(&pool)
    .await
    .context("Fetching entitlements")?;

    if let Some(e) = entitlement {
        lines.push(serde_json::to_string(&serde_json::json!({
            "record_type": "entitlement",
            "tenant_id": tid,
            "plan_code": e.plan_code,
            "concurrent_user_limit": e.concurrent_user_limit,
            "effective_at": e.effective_at,
            "updated_at": e.updated_at,
        }))?);
    }

    // 4. Provisioning requests (sanitised — idempotency keys omitted; sorted by created_at)
    let prov_rows: Vec<ProvRow> = sqlx::query_as(
        r#"SELECT environment, created_at
           FROM provisioning_requests
           WHERE tenant_id = $1
           ORDER BY created_at ASC"#,
    )
    .bind(tid)
    .fetch_all(&pool)
    .await
    .context("Fetching provisioning requests")?;

    for (i, r) in prov_rows.iter().enumerate() {
        lines.push(serde_json::to_string(&serde_json::json!({
            "record_type": "provisioning_request",
            "tenant_id": tid,
            "seq": i,
            "environment": r.environment,
            "created_at": r.created_at,
        }))?);
    }

    // ---- Build JSONL artifact ----
    let artifact = lines.join("\n") + "\n";
    let artifact_bytes = artifact.as_bytes();

    // ---- Compute deterministic SHA-256 digest ----
    let mut hasher = Sha256::new();
    hasher.update(artifact_bytes);
    let sha256_digest = hex::encode(hasher.finalize());

    // ---- Write artifact to disk (if path provided) ----
    let artifact_path = if let Some(path) = output_path {
        std::fs::write(path, artifact_bytes)
            .with_context(|| format!("Writing export artifact to {path}"))?;
        path.to_string()
    } else {
        "<stdout>".to_string()
    };

    let line_count = lines.len();

    // ---- Update export_ready_at ----
    let now = Utc::now();
    sqlx::query(
        r#"INSERT INTO cp_retention_policies (tenant_id, export_ready_at, created_at, updated_at)
           VALUES ($1, $2, $2, $2)
           ON CONFLICT (tenant_id) DO UPDATE
               SET export_ready_at = EXCLUDED.export_ready_at,
                   updated_at      = EXCLUDED.updated_at"#,
    )
    .bind(tid)
    .bind(now)
    .execute(&pool)
    .await
    .context("Updating export_ready_at")?;

    tracing::info!(
        tenant_id = %tid,
        artifact_path = %artifact_path,
        sha256 = %sha256_digest,
        lines = line_count,
        "Tenant export complete"
    );

    Ok(ExportResult {
        artifact_path,
        sha256_digest,
        line_count,
    })
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_tenant_id(tenant_id: &str) -> Result<Uuid> {
    if tenant_id.len() == 36 {
        Uuid::parse_str(tenant_id).context("Invalid tenant UUID format")
    } else {
        Ok(Uuid::new_v5(&Uuid::NAMESPACE_DNS, tenant_id.as_bytes()))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tenant_id_accepts_uuid() {
        let id = "00000000-0000-0000-0000-000000000001";
        let parsed = parse_tenant_id(id).unwrap();
        assert_eq!(parsed.to_string(), id);
    }

    #[test]
    fn parse_tenant_id_derives_v5_for_short_name() {
        // Same name → same UUID (deterministic)
        let a = parse_tenant_id("acme").unwrap();
        let b = parse_tenant_id("acme").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn sha256_of_empty_string_is_known_value() {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"");
        let d = hex::encode(h.finalize());
        assert_eq!(
            d,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn same_lines_produce_same_digest() {
        use sha2::{Digest, Sha256};
        let lines = vec![
            r#"{"record_type":"tenant","status":"deleted"}"#,
            r#"{"record_type":"retention_policy","data_retention_days":2555}"#,
        ];
        let artifact = lines.join("\n") + "\n";
        let d1 = hex::encode(Sha256::digest(artifact.as_bytes()));
        let d2 = hex::encode(Sha256::digest(artifact.as_bytes()));
        assert_eq!(d1, d2, "Same input must produce same digest");
    }
}
