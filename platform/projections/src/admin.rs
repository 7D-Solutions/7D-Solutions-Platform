//! Shared admin query functions for projection management
//!
//! Provides standardized, framework-agnostic query functions that modules
//! wire into their HTTP admin endpoints. All operations are idempotent and
//! read-only (except rebuild triggers).
//!
//! # Endpoints Pattern
//!
//! Each module mounts these under `POST /api/{module}/admin/{action}`:
//! - `projection-status` — cursor position for a named projection
//! - `consistency-check` — versioned digest for integrity verification
//! - `projections`       — list all known projections in this database
//!
//! Auth is handled at the HTTP layer via `ADMIN_TOKEN` env var.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

// ── Request types ────────────────────────────────────────────────────────────

/// Request body for projection status queries.
#[derive(Debug, Deserialize)]
pub struct ProjectionStatusRequest {
    pub projection_name: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

/// Request body for consistency check.
#[derive(Debug, Deserialize)]
pub struct ConsistencyCheckRequest {
    pub projection_name: String,
    /// Column(s) to order by for deterministic digest (default: "tenant_id").
    #[serde(default = "default_order_by")]
    pub order_by: String,
}

fn default_order_by() -> String {
    "tenant_id".to_string()
}

// ── Response types ───────────────────────────────────────────────────────────

/// Cursor status for a projection/tenant pair.
#[derive(Debug, Serialize)]
pub struct CursorStatus {
    pub projection_name: String,
    pub tenant_id: String,
    pub events_processed: i64,
    pub last_event_id: String,
    pub last_event_occurred_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full projection status response (may contain multiple tenant cursors).
#[derive(Debug, Serialize)]
pub struct ProjectionStatusResponse {
    pub projection_name: String,
    pub cursors: Vec<CursorStatus>,
    pub status: &'static str,
}

/// Consistency check result.
#[derive(Debug, Serialize)]
pub struct ConsistencyCheckResponse {
    pub projection_name: String,
    pub table_exists: bool,
    pub row_count: i64,
    pub digest: String,
    pub digest_version: String,
    pub order_by: String,
    pub checked_at: DateTime<Utc>,
    pub status: &'static str,
}

/// Summary of a single projection in the listing.
#[derive(Debug, Serialize)]
pub struct ProjectionSummary {
    pub projection_name: String,
    pub tenant_count: i64,
    pub total_events_processed: i64,
    pub last_updated: Option<DateTime<Utc>>,
}

/// Response for listing all projections.
#[derive(Debug, Serialize)]
pub struct ProjectionListResponse {
    pub projections: Vec<ProjectionSummary>,
    pub status: &'static str,
}

// ── Query functions ──────────────────────────────────────────────────────────

/// Query cursor status for a projection, optionally filtered by tenant.
pub async fn query_projection_status(
    pool: &PgPool,
    req: &ProjectionStatusRequest,
) -> Result<ProjectionStatusResponse, String> {
    // Check if cursor table exists
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'projection_cursors')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if !table_exists {
        return Ok(ProjectionStatusResponse {
            projection_name: req.projection_name.clone(),
            cursors: vec![],
            status: "no_cursor_table",
        });
    }

    let cursors = if let Some(tid) = &req.tenant_id {
        sqlx::query_as::<
            _,
            (
                String,
                String,
                i64,
                uuid::Uuid,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            "SELECT projection_name, tenant_id, events_processed, \
                    last_event_id, last_event_occurred_at, updated_at \
             FROM projection_cursors \
             WHERE projection_name = $1 AND tenant_id = $2 \
             ORDER BY tenant_id",
        )
        .bind(&req.projection_name)
        .bind(tid)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("DB error: {e}"))?
    } else {
        sqlx::query_as::<
            _,
            (
                String,
                String,
                i64,
                uuid::Uuid,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            "SELECT projection_name, tenant_id, events_processed, \
                    last_event_id, last_event_occurred_at, updated_at \
             FROM projection_cursors \
             WHERE projection_name = $1 \
             ORDER BY tenant_id",
        )
        .bind(&req.projection_name)
        .fetch_all(pool)
        .await
        .map_err(|e| format!("DB error: {e}"))?
    };

    let cursor_statuses: Vec<CursorStatus> = cursors
        .into_iter()
        .map(|(pn, tid, ep, eid, occ, upd)| CursorStatus {
            projection_name: pn,
            tenant_id: tid,
            events_processed: ep,
            last_event_id: eid.to_string(),
            last_event_occurred_at: occ,
            updated_at: upd,
        })
        .collect();

    let status = if cursor_statuses.is_empty() {
        "no_cursors"
    } else {
        "ok"
    };

    Ok(ProjectionStatusResponse {
        projection_name: req.projection_name.clone(),
        cursors: cursor_statuses,
        status,
    })
}

/// Compute a consistency digest for a projection table.
pub async fn query_consistency_check(
    pool: &PgPool,
    req: &ConsistencyCheckRequest,
) -> Result<ConsistencyCheckResponse, String> {
    // Check if table exists
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = $1)",
    )
    .bind(&req.projection_name)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if !table_exists {
        return Ok(ConsistencyCheckResponse {
            projection_name: req.projection_name.clone(),
            table_exists: false,
            row_count: 0,
            digest: String::new(),
            digest_version: crate::digest::DIGEST_VERSION.to_string(),
            order_by: req.order_by.clone(),
            checked_at: Utc::now(),
            status: "table_not_found",
        });
    }

    let table = crate::validate::validate_projection_name(&req.projection_name)
        .map_err(|e| format!("Validation error: {e}"))?;
    let order = crate::validate::validate_order_column(&req.order_by)
        .map_err(|e| format!("Validation error: {e}"))?;

    let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table))
        .fetch_one(pool)
        .await
        .map_err(|e| format!("DB error: {e}"))?;

    let versioned = crate::digest::compute_versioned_digest(pool, table, order)
        .await
        .map_err(|e| format!("Digest error: {e}"))?;

    Ok(ConsistencyCheckResponse {
        projection_name: req.projection_name.clone(),
        table_exists: true,
        row_count,
        digest: versioned.to_string(),
        digest_version: versioned.version,
        order_by: req.order_by.clone(),
        checked_at: Utc::now(),
        status: "ok",
    })
}

/// List all known projections from the cursor table.
pub async fn query_projection_list(pool: &PgPool) -> Result<ProjectionListResponse, String> {
    let table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = 'projection_cursors')",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    if !table_exists {
        return Ok(ProjectionListResponse {
            projections: vec![],
            status: "no_cursor_table",
        });
    }

    let rows = sqlx::query_as::<_, (String, i64, i64, Option<DateTime<Utc>>)>(
        "SELECT projection_name, COUNT(*)::BIGINT as tenant_count, \
                COALESCE(SUM(events_processed), 0)::BIGINT as total_events, \
                MAX(updated_at) as last_updated \
         FROM projection_cursors \
         GROUP BY projection_name \
         ORDER BY projection_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("DB error: {e}"))?;

    let projections: Vec<ProjectionSummary> = rows
        .into_iter()
        .map(|(name, tc, te, lu)| ProjectionSummary {
            projection_name: name,
            tenant_count: tc,
            total_events_processed: te,
            last_updated: lu,
        })
        .collect();

    Ok(ProjectionListResponse {
        projections,
        status: "ok",
    })
}

// ── Admin token verification ─────────────────────────────────────────────────

/// Verify the admin token from request headers.
///
/// Returns Ok(()) if the token is valid, Err(message) otherwise.
/// Uses `ADMIN_TOKEN` env var. If not set, all admin requests are rejected.
pub fn verify_admin_token(provided: Option<&str>) -> Result<(), &'static str> {
    let expected = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    verify_token_against(provided, &expected)
}

/// Inner token comparison (testable without env vars).
fn verify_token_against(provided: Option<&str>, expected: &str) -> Result<(), &'static str> {
    if expected.is_empty() {
        return Err("ADMIN_TOKEN not configured; admin endpoints disabled");
    }
    match provided {
        Some(token) if token == expected => Ok(()),
        _ => Err("Invalid or missing admin token"),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_token_rejects_when_not_configured() {
        let result = verify_token_against(Some("any-token"), "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not configured"));
    }

    #[test]
    fn test_admin_token_rejects_wrong_token() {
        let result = verify_token_against(Some("wrong-token"), "correct-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_admin_token_accepts_correct_token() {
        let result = verify_token_against(Some("correct-token"), "correct-token");
        assert!(result.is_ok());
    }

    #[test]
    fn test_admin_token_rejects_missing_header() {
        let result = verify_token_against(None, "correct-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_default_order_by() {
        assert_eq!(default_order_by(), "tenant_id");
    }

    #[test]
    fn test_consistency_request_deserialize() {
        let json = r#"{"projection_name":"customer_balances"}"#;
        let req: ConsistencyCheckRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.projection_name, "customer_balances");
        assert_eq!(req.order_by, "tenant_id");
    }

    #[test]
    fn test_projection_status_request_deserialize() {
        let json = r#"{"projection_name":"invoice_totals","tenant_id":"acme"}"#;
        let req: ProjectionStatusRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.projection_name, "invoice_totals");
        assert_eq!(req.tenant_id, Some("acme".to_string()));
    }

    #[test]
    fn test_consistency_request_rejects_sql_injection() {
        let json = r#"{"projection_name":"users; DROP TABLE credentials; --"}"#;
        let req: ConsistencyCheckRequest = serde_json::from_str(json).unwrap();
        let result = crate::validate::validate_projection_name(&req.projection_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_consistency_request_rejects_unknown_table() {
        let json = r#"{"projection_name":"not_a_real_table"}"#;
        let req: ConsistencyCheckRequest = serde_json::from_str(json).unwrap();
        let result = crate::validate::validate_projection_name(&req.projection_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_consistency_request_accepts_allowlisted_table() {
        let json = r#"{"projection_name":"projection_cursors"}"#;
        let req: ConsistencyCheckRequest = serde_json::from_str(json).unwrap();
        let result = crate::validate::validate_projection_name(&req.projection_name);
        assert!(result.is_ok());
    }
}
