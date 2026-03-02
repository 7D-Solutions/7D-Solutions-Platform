//! Versioned digest computation for projection certification
//!
//! This module provides deterministic, versioned digest computation for projections.
//! Digests are used to certify that rebuilds produce identical state.

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::rebuild::{RebuildError, RebuildResult};

/// Current digest algorithm version
pub const DIGEST_VERSION: &str = "v1";

/// Compute a versioned, deterministic digest of a projection
///
/// The digest includes:
/// - Algorithm version (for future compatibility)
/// - Row count
/// - SHA-256 hash of ordered row data
///
/// # Arguments
///
/// * `pool` - Database connection pool
/// * `table_name` - Name of the projection table
/// * `order_by` - Column(s) to order by for deterministic iteration
///
/// # Returns
///
/// A versioned digest string in the format: `<version>:<row_count>:<hash>`
///
/// # Example
///
/// ```text
/// v1:1000:a1b2c3d4e5f6...
/// ```
pub async fn compute_versioned_digest(
    pool: &PgPool,
    table_name: &str,
    order_by: &str,
) -> RebuildResult<VersionedDigest> {
    // Validate identifiers before using in dynamic SQL
    crate::validate::validate_projection_name(table_name)
        .map_err(|e| RebuildError::Failed(e.to_string()))?;
    crate::validate::validate_order_column(order_by)
        .map_err(|e| RebuildError::Failed(e.to_string()))?;

    // Get row count
    let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", table_name))
        .fetch_one(pool)
        .await?;

    // Compute content hash
    let content_hash = compute_content_hash(pool, table_name, order_by).await?;

    Ok(VersionedDigest {
        version: DIGEST_VERSION.to_string(),
        row_count,
        content_hash,
    })
}

/// Compute SHA-256 hash of table contents in deterministic order
async fn compute_content_hash(
    pool: &PgPool,
    table_name: &str,
    order_by: &str,
) -> RebuildResult<String> {
    // Query all rows in deterministic order
    let query = format!("SELECT * FROM {} ORDER BY {}", table_name, order_by);

    let mut rows = sqlx::query(&query).fetch(pool);

    let mut hasher = Sha256::new();

    // Stream rows and hash them
    use sqlx::Row;
    while let Some(row) = {
        use futures::StreamExt;
        rows.next().await
    } {
        let row = row?;

        // Hash each column value in order
        for i in 0..row.len() {
            let column_value = row.try_get_raw(i)?;
            hasher.update(column_value.as_bytes().unwrap_or(&[]));
        }
    }

    // Finalize hash
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// A versioned digest with metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedDigest {
    /// Digest algorithm version
    pub version: String,

    /// Total number of rows in the projection
    pub row_count: i64,

    /// SHA-256 hash of the projection contents
    pub content_hash: String,
}

impl VersionedDigest {
    /// Create a new versioned digest
    pub fn new(version: String, row_count: i64, content_hash: String) -> Self {
        Self {
            version,
            row_count,
            content_hash,
        }
    }

    /// Parse from a compact string representation
    pub fn from_string(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return None;
        }

        let row_count = parts[1].parse::<i64>().ok()?;

        Some(Self {
            version: parts[0].to_string(),
            row_count,
            content_hash: parts[2].to_string(),
        })
    }
}

impl std::fmt::Display for VersionedDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.version, self.row_count, self.content_hash
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_versioned_digest_to_string() {
        let digest = VersionedDigest::new("v1".to_string(), 1000, "abc123def456".to_string());

        assert_eq!(digest.to_string(), "v1:1000:abc123def456");
    }

    #[test]
    fn test_versioned_digest_from_string() {
        let digest_str = "v1:1000:abc123def456";
        let digest = VersionedDigest::from_string(digest_str).unwrap();

        assert_eq!(digest.version, "v1");
        assert_eq!(digest.row_count, 1000);
        assert_eq!(digest.content_hash, "abc123def456");
    }

    #[test]
    fn test_versioned_digest_roundtrip() {
        let original = VersionedDigest::new("v1".to_string(), 5000, "fedcba987654".to_string());

        let serialized = original.to_string();
        let deserialized = VersionedDigest::from_string(&serialized).unwrap();

        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_versioned_digest_display() {
        let digest = VersionedDigest::new("v2".to_string(), 42, "hash123".to_string());

        assert_eq!(format!("{}", digest), "v2:42:hash123");
    }
}
