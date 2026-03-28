//! Evidence Pack Generator
//!
//! Bundles period close artifacts into a single tamper-evident evidence pack:
//! - Sealed period snapshot (from period_summary_snapshots)
//! - Close hash from accounting_periods
//! - Reopen history (if any)
//! - References to compliance-export manifest with checksums
//!
//! The pack is a self-contained JSON file suitable for audit review.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use std::path::Path;
use uuid::Uuid;

/// Complete evidence pack for a closed period
#[derive(Debug, Serialize, Deserialize)]
pub struct EvidencePack {
    pub pack_version: &'static str,
    pub generated_at: DateTime<Utc>,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub period_start: String,
    pub period_end: String,
    pub close_state: CloseState,
    pub snapshot: Option<SnapshotSummary>,
    pub reopen_history: Vec<ReopenEntry>,
    pub export_manifest_ref: Option<ManifestReference>,
    pub pack_hash: String,
}

/// Close state of the period at pack generation time
#[derive(Debug, Serialize, Deserialize)]
pub struct CloseState {
    pub is_closed: bool,
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<String>,
    pub close_hash: Option<String>,
    pub reopen_count: i32,
}

/// Sealed snapshot summary from period_summary_snapshots
#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub currencies: Vec<CurrencySnapshotEntry>,
    pub total_journal_count: i64,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CurrencySnapshotEntry {
    pub currency: String,
    pub journal_count: i32,
    pub line_count: i32,
    pub total_debits_minor: i64,
    pub total_credits_minor: i64,
}

/// Reopen audit trail entry
#[derive(Debug, Serialize, Deserialize)]
pub struct ReopenEntry {
    pub request_id: Uuid,
    pub requested_by: String,
    pub reason: String,
    pub prior_close_hash: String,
    pub status: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Reference to a compliance-export manifest on disk
#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestReference {
    pub manifest_path: String,
    pub manifest_checksum: String,
}

/// Generate an evidence pack for a closed period.
///
/// Queries GL database for:
/// - Period close state (accounting_periods)
/// - Sealed snapshot (period_summary_snapshots)
/// - Reopen history (period_reopen_requests)
///
/// Optionally references a compliance-export manifest file on disk.
pub async fn generate_evidence_pack(
    gl_pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    manifest_path: Option<&str>,
) -> Result<EvidencePack> {
    // 1. Query period close state
    let period_row = sqlx::query(
        r#"
        SELECT period_start, period_end, closed_at, closed_by, close_hash,
               COALESCE(reopen_count, 0) as reopen_count
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(gl_pool)
    .await
    .context("Failed to query period")?
    .context(format!(
        "Period {} not found for tenant {}",
        period_id, tenant_id
    ))?;

    let period_start: chrono::NaiveDate = period_row.get("period_start");
    let period_end: chrono::NaiveDate = period_row.get("period_end");
    let closed_at: Option<DateTime<Utc>> = period_row.get("closed_at");
    let closed_by: Option<String> = period_row.get("closed_by");
    let close_hash: Option<String> = period_row.get("close_hash");
    let reopen_count: i32 = period_row.get("reopen_count");

    let close_state = CloseState {
        is_closed: closed_at.is_some(),
        closed_at,
        closed_by,
        close_hash,
        reopen_count,
    };

    // 2. Query sealed snapshot
    let snapshot_rows = sqlx::query(
        r#"
        SELECT currency, journal_count, line_count,
               total_debits_minor, total_credits_minor
        FROM period_summary_snapshots
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY currency
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(gl_pool)
    .await
    .context("Failed to query snapshots")?;

    let snapshot = if snapshot_rows.is_empty() {
        None
    } else {
        let currencies: Vec<CurrencySnapshotEntry> = snapshot_rows
            .iter()
            .map(|r| CurrencySnapshotEntry {
                currency: r.get("currency"),
                journal_count: r.get("journal_count"),
                line_count: r.get("line_count"),
                total_debits_minor: r.get("total_debits_minor"),
                total_credits_minor: r.get("total_credits_minor"),
            })
            .collect();

        let total_journal_count: i64 = currencies.iter().map(|c| c.journal_count as i64).sum();
        let total_debits_minor: i64 = currencies.iter().map(|c| c.total_debits_minor).sum();
        let total_credits_minor: i64 = currencies.iter().map(|c| c.total_credits_minor).sum();

        Some(SnapshotSummary {
            currencies,
            total_journal_count,
            total_debits_minor,
            total_credits_minor,
        })
    };

    // 3. Query reopen history
    let reopen_rows = sqlx::query(
        r#"
        SELECT id, requested_by, reason, prior_close_hash, status,
               COALESCE(approved_by, rejected_by) as decided_by,
               COALESCE(approved_at, rejected_at) as decided_at,
               created_at
        FROM period_reopen_requests
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(gl_pool)
    .await
    .context("Failed to query reopen history")?;

    let reopen_history: Vec<ReopenEntry> = reopen_rows
        .iter()
        .map(|r| ReopenEntry {
            request_id: r.get("id"),
            requested_by: r.get("requested_by"),
            reason: r.get("reason"),
            prior_close_hash: r.get("prior_close_hash"),
            status: r.get("status"),
            decided_by: r.get("decided_by"),
            decided_at: r.get("decided_at"),
            created_at: r.get("created_at"),
        })
        .collect();

    // 4. Reference compliance-export manifest if provided
    let export_manifest_ref = if let Some(path) = manifest_path {
        let manifest_file = Path::new(path);
        if manifest_file.exists() {
            let content = std::fs::read(manifest_file).context("Failed to read manifest file")?;
            let mut hasher = Sha256::new();
            hasher.update(&content);
            let checksum = hex::encode(hasher.finalize());
            Some(ManifestReference {
                manifest_path: path.to_string(),
                manifest_checksum: checksum,
            })
        } else {
            None
        }
    } else {
        None
    };

    // 5. Compute pack hash over all content
    let now = Utc::now();
    let mut pack = EvidencePack {
        pack_version: "1.0",
        generated_at: now,
        tenant_id: tenant_id.to_string(),
        period_id,
        period_start: period_start.to_string(),
        period_end: period_end.to_string(),
        close_state,
        snapshot,
        reopen_history,
        export_manifest_ref,
        pack_hash: String::new(), // placeholder
    };

    // Hash the pack content (excluding pack_hash itself)
    let json_for_hash = serde_json::to_string(&pack)?;
    let mut hasher = Sha256::new();
    hasher.update(json_for_hash.as_bytes());
    pack.pack_hash = hex::encode(hasher.finalize());

    Ok(pack)
}

/// Write an evidence pack to a JSON file
pub fn write_evidence_pack(pack: &EvidencePack, output_path: &Path) -> Result<()> {
    let file = std::fs::File::create(output_path)?;
    let writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(writer, pack)?;
    Ok(())
}

/// Calculate SHA-256 hash of content (reusable for pack hash computation)
pub fn compute_sha256(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_close_state_serialization_roundtrip() {
        let state = CloseState {
            is_closed: true,
            closed_at: Some(Utc::now()),
            closed_by: Some("controller@example.com".to_string()),
            close_hash: Some("a1b2c3d4e5f6".to_string()),
            reopen_count: 1,
        };

        let json = serde_json::to_string(&state).unwrap();
        let parsed: CloseState = serde_json::from_str(&json).unwrap();

        assert_eq!(state.is_closed, parsed.is_closed);
        assert_eq!(state.closed_by, parsed.closed_by);
        assert_eq!(state.close_hash, parsed.close_hash);
        assert_eq!(state.reopen_count, parsed.reopen_count);
    }

    #[test]
    fn test_snapshot_summary_serialization_roundtrip() {
        let snapshot = SnapshotSummary {
            currencies: vec![
                CurrencySnapshotEntry {
                    currency: "USD".to_string(),
                    journal_count: 100,
                    line_count: 250,
                    total_debits_minor: 1_000_000,
                    total_credits_minor: 1_000_000,
                },
                CurrencySnapshotEntry {
                    currency: "EUR".to_string(),
                    journal_count: 50,
                    line_count: 120,
                    total_debits_minor: 500_000,
                    total_credits_minor: 500_000,
                },
            ],
            total_journal_count: 150,
            total_debits_minor: 1_500_000,
            total_credits_minor: 1_500_000,
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let parsed: SnapshotSummary = serde_json::from_str(&json).unwrap();

        assert_eq!(snapshot.currencies.len(), parsed.currencies.len());
        assert_eq!(snapshot.total_journal_count, parsed.total_journal_count);
        assert_eq!(
            snapshot.currencies[0].currency,
            parsed.currencies[0].currency
        );
    }

    #[test]
    fn test_evidence_pack_serialization() {
        // EvidencePack has &'static str field, so we test serialization to JSON
        // and verify the structure rather than roundtrip deserialization
        let period_id = Uuid::new_v4();
        let pack = EvidencePack {
            pack_version: "1.0",
            generated_at: Utc::now(),
            tenant_id: "tenant-123".to_string(),
            period_id,
            period_start: "2024-01-01".to_string(),
            period_end: "2024-01-31".to_string(),
            close_state: CloseState {
                is_closed: true,
                closed_at: Some(Utc::now()),
                closed_by: Some("admin".to_string()),
                close_hash: Some("hash123".to_string()),
                reopen_count: 0,
            },
            snapshot: Some(SnapshotSummary {
                currencies: vec![CurrencySnapshotEntry {
                    currency: "USD".to_string(),
                    journal_count: 50,
                    line_count: 100,
                    total_debits_minor: 250_000,
                    total_credits_minor: 250_000,
                }],
                total_journal_count: 50,
                total_debits_minor: 250_000,
                total_credits_minor: 250_000,
            }),
            reopen_history: vec![],
            export_manifest_ref: None,
            pack_hash: "placeholder".to_string(),
        };

        let json = serde_json::to_string(&pack).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["tenant_id"], "tenant-123");
        assert_eq!(parsed["period_id"], period_id.to_string());
        assert_eq!(parsed["period_start"], "2024-01-01");
        assert_eq!(parsed["pack_version"], "1.0");
        assert_eq!(parsed["close_state"]["is_closed"], true);
        assert_eq!(parsed["close_state"]["close_hash"], "hash123");
        assert_eq!(parsed["snapshot"]["total_journal_count"], 50);
    }

    #[test]
    fn test_write_evidence_pack_creates_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("evidence_pack.json");

        let pack = EvidencePack {
            pack_version: "1.0",
            generated_at: Utc::now(),
            tenant_id: "test-tenant".to_string(),
            period_id: Uuid::new_v4(),
            period_start: "2024-02-01".to_string(),
            period_end: "2024-02-29".to_string(),
            close_state: CloseState {
                is_closed: false,
                closed_at: None,
                closed_by: None,
                close_hash: None,
                reopen_count: 0,
            },
            snapshot: None,
            reopen_history: vec![],
            export_manifest_ref: None,
            pack_hash: "test-hash".to_string(),
        };

        write_evidence_pack(&pack, &path).unwrap();

        // Verify file exists and is valid JSON
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(parsed["tenant_id"], "test-tenant");
        assert_eq!(parsed["pack_version"], "1.0");
        assert_eq!(parsed["close_state"]["is_closed"], false);
    }

    #[test]
    fn test_compute_sha256() {
        // Known SHA-256 hash of "hello"
        let hash = compute_sha256(b"hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Same input produces same output
        let hash2 = compute_sha256(b"hello");
        assert_eq!(hash, hash2);

        // Different input produces different output
        let hash3 = compute_sha256(b"world");
        assert_ne!(hash, hash3);
    }

    #[test]
    fn test_reopen_entry_serialization_roundtrip() {
        let entry = ReopenEntry {
            request_id: Uuid::new_v4(),
            requested_by: "user@example.com".to_string(),
            reason: "Discovered missing journal entry".to_string(),
            prior_close_hash: "abc123def456".to_string(),
            status: "approved".to_string(),
            decided_by: Some("manager@example.com".to_string()),
            decided_at: Some(Utc::now()),
            created_at: Utc::now(),
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ReopenEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(entry.request_id, parsed.request_id);
        assert_eq!(entry.reason, parsed.reason);
        assert_eq!(entry.status, parsed.status);
    }

    #[test]
    fn test_manifest_reference_serialization_roundtrip() {
        let reference = ManifestReference {
            manifest_path: "/exports/2024/01/manifest.json".to_string(),
            manifest_checksum: "sha256-hash-here".to_string(),
        };

        let json = serde_json::to_string(&reference).unwrap();
        let parsed: ManifestReference = serde_json::from_str(&json).unwrap();

        assert_eq!(reference.manifest_path, parsed.manifest_path);
        assert_eq!(reference.manifest_checksum, parsed.manifest_checksum);
    }
}
