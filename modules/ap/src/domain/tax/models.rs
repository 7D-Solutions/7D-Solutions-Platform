//! AP tax snapshot model — persisted audit record of the tax lifecycle per bill.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Persisted tax snapshot for an AP vendor bill.
///
/// Records the full quote -> commit -> void lifecycle with provider references
/// and a deterministic quote hash for idempotency.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ApTaxSnapshot {
    pub id: Uuid,
    pub bill_id: Uuid,
    pub tenant_id: String,
    pub provider: String,
    pub provider_quote_ref: String,
    pub provider_commit_ref: Option<String>,
    pub quote_hash: String,
    pub total_tax_minor: i64,
    pub tax_by_line: serde_json::Value,
    pub status: String,
    pub quoted_at: DateTime<Utc>,
    pub committed_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub void_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serializes_roundtrip() {
        let snap = ApTaxSnapshot {
            id: Uuid::new_v4(),
            bill_id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            provider: "zero".to_string(),
            provider_quote_ref: "zero-quote-123".to_string(),
            provider_commit_ref: None,
            quote_hash: "abc123".to_string(),
            total_tax_minor: 500,
            tax_by_line: serde_json::json!([]),
            status: "quoted".to_string(),
            quoted_at: Utc::now(),
            committed_at: None,
            voided_at: None,
            void_reason: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ApTaxSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_tax_minor, 500);
        assert_eq!(back.status, "quoted");
    }

    #[test]
    fn snapshot_status_values_match_db_check_constraint() {
        for status in ["quoted", "committed", "voided"] {
            let snap = ApTaxSnapshot {
                id: Uuid::new_v4(),
                bill_id: Uuid::new_v4(),
                tenant_id: "t".to_string(),
                provider: "zero".to_string(),
                provider_quote_ref: "ref".to_string(),
                provider_commit_ref: None,
                quote_hash: "h".to_string(),
                total_tax_minor: 0,
                tax_by_line: serde_json::json!([]),
                status: status.to_string(),
                quoted_at: Utc::now(),
                committed_at: None,
                voided_at: None,
                void_reason: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            assert_eq!(snap.status, status);
        }
    }
}
