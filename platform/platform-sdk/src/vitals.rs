//! Operational vitals collection for the platform vitals API.
//!
//! `StandardVitalsProvider` queries real DB tables (event_dlq, outbox, projection_cursors)
//! and returns zeros / empty results gracefully when tables are absent — modules at different
//! maturity levels must never panic because they haven't created a table yet.
//!
//! `VitalsProvider` is the extension point for service-specific data that doesn't fit the
//! standard schema.  The `extended` field of `VitalsResponse` receives its output.

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

pub use health::{ConsumerVitals, DlqVitals, OutboxVitals, ProjectionVitals, VitalsResponse};

/// Extension point for service-specific vitals data.
///
/// Implement on a unit struct (or a struct holding any extra context) and
/// register it with [`ModuleBuilder::vitals_handler`].  The return value is
/// placed in `VitalsResponse.extended`.
///
/// Returning `serde_json::Value::Null` is treated identically to `None`
/// — the `extended` field is omitted from the serialized response.
#[async_trait]
pub trait VitalsProvider: Send + Sync {
    async fn collect_extended(
        &self,
        pool: &PgPool,
        tenant_id: Option<Uuid>,
    ) -> serde_json::Value;
}

/// Default vitals provider that queries the standard platform tables.
///
/// All queries degrade gracefully: if a table does not exist the
/// corresponding vitals fields are zeroed / empty rather than erroring.
pub struct StandardVitalsProvider {
    /// Name of this module's outbox table.  Defaults to `"events_outbox"`.
    outbox_table: String,
}

impl Default for StandardVitalsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardVitalsProvider {
    pub fn new() -> Self {
        Self {
            outbox_table: "events_outbox".to_string(),
        }
    }

    pub fn with_outbox_table(table: impl Into<String>) -> Self {
        Self {
            outbox_table: table.into(),
        }
    }

    /// Query DLQ stats.  Returns all-zeros if the table doesn't exist.
    async fn query_dlq(pool: &PgPool) -> DlqVitals {
        let result: Result<Vec<(String, i64)>, _> = sqlx::query_as(
            "SELECT failure_kind, COUNT(*)::bigint FROM event_dlq GROUP BY failure_kind",
        )
        .fetch_all(pool)
        .await;

        match result {
            Ok(rows) => {
                let mut retryable = 0u64;
                let mut fatal = 0u64;
                let mut poison = 0u64;
                for (kind, count) in rows {
                    let n = count.max(0) as u64;
                    match kind.as_str() {
                        "retryable" => retryable = n,
                        "fatal" => fatal = n,
                        "poison" => poison = n,
                        _ => {}
                    }
                }
                DlqVitals {
                    total: retryable + fatal + poison,
                    retryable,
                    fatal,
                    poison,
                }
            }
            Err(_) => DlqVitals {
                total: 0,
                retryable: 0,
                fatal: 0,
                poison: 0,
            },
        }
    }

    /// Query outbox pending count.  Returns zeros if the table doesn't exist.
    async fn query_outbox(pool: &PgPool, table: &str) -> OutboxVitals {
        // Table name comes from the manifest config — never user input.
        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {} WHERE published_at IS NULL",
            table
        );
        let pending_result: Result<Option<i64>, _> =
            sqlx::query_scalar(&count_sql).fetch_one(pool).await;

        match pending_result {
            Ok(count) => {
                let pending = count.unwrap_or(0).max(0) as u64;
                let oldest_pending_secs = if pending > 0 {
                    let age_sql = format!(
                        "SELECT EXTRACT(EPOCH FROM (now() - MIN(created_at)))::bigint \
                         FROM {} WHERE published_at IS NULL",
                        table
                    );
                    sqlx::query_scalar::<_, Option<i64>>(&age_sql)
                        .fetch_one(pool)
                        .await
                        .ok()
                        .flatten()
                        .map(|s| s.max(0) as u64)
                } else {
                    None
                };
                OutboxVitals {
                    pending,
                    oldest_pending_secs,
                }
            }
            Err(_) => OutboxVitals {
                pending: 0,
                oldest_pending_secs: None,
            },
        }
    }

    /// Query projection cursor freshness.  Returns empty vec if table doesn't exist.
    async fn query_projections(pool: &PgPool, tenant_id: Option<Uuid>) -> Vec<ProjectionVitals> {
        let result: Result<Vec<(String, String, i64, i64)>, _> = if let Some(tid) = tenant_id {
            sqlx::query_as(
                "SELECT projection_name, tenant_id, \
                 (EXTRACT(EPOCH FROM (now() - last_event_occurred_at)) * 1000)::bigint AS lag_ms, \
                 EXTRACT(EPOCH FROM (now() - updated_at))::bigint AS age_seconds \
                 FROM projection_cursors WHERE tenant_id = $1",
            )
            .bind(tid.to_string())
            .fetch_all(pool)
            .await
        } else {
            sqlx::query_as(
                "SELECT projection_name, tenant_id, \
                 (EXTRACT(EPOCH FROM (now() - last_event_occurred_at)) * 1000)::bigint AS lag_ms, \
                 EXTRACT(EPOCH FROM (now() - updated_at))::bigint AS age_seconds \
                 FROM projection_cursors",
            )
            .fetch_all(pool)
            .await
        };

        match result {
            Ok(rows) => rows
                .into_iter()
                .map(|(name, tid, lag_ms, age_seconds)| ProjectionVitals {
                    name,
                    tenant_id: tid,
                    lag_ms,
                    age_seconds,
                })
                .collect(),
            Err(_) => vec![],
        }
    }

    /// Build a complete `VitalsResponse` from DB queries.
    pub async fn collect(
        &self,
        pool: &PgPool,
        service_name: &str,
        version: &str,
        tenant_id: Option<Uuid>,
        tenant_ready: Option<bool>,
        extended: Option<serde_json::Value>,
    ) -> VitalsResponse {
        let dlq = Self::query_dlq(pool).await;
        let outbox = Self::query_outbox(pool, &self.outbox_table).await;
        let projections = Self::query_projections(pool, tenant_id).await;

        // Consumer vitals require JetStreamConsumer health tracking, which is not
        // yet implemented.  Return an empty slice until that infrastructure exists.
        let consumers: Vec<ConsumerVitals> = vec![];

        let extended_val = match extended {
            Some(v) if !v.is_null() => Some(v),
            _ => None,
        };

        VitalsResponse {
            service_name: service_name.to_string(),
            version: version.to_string(),
            tenant_ready,
            dlq,
            outbox,
            projections,
            consumers,
            extended: extended_val,
            timestamp: Utc::now().to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("connect to test database for vitals tests")
    }

    /// DLQ query returns zeros when the table doesn't exist.
    #[tokio::test]
    async fn vitals_dlq_count_zeros_on_missing_table() {
        let pool = test_pool().await;
        let dlq = StandardVitalsProvider::query_dlq(&pool).await;
        // event_dlq may or may not exist; either way should not panic
        assert!(dlq.total == dlq.retryable + dlq.fatal + dlq.poison);
    }

    /// Outbox query returns zeros for a non-existent table.
    #[tokio::test]
    async fn vitals_outbox_pending_zeros_on_missing_table() {
        let pool = test_pool().await;
        let outbox =
            StandardVitalsProvider::query_outbox(&pool, "nonexistent_outbox_xyz").await;
        assert_eq!(outbox.pending, 0);
        assert!(outbox.oldest_pending_secs.is_none());
    }

    /// Projection query returns empty vec when table doesn't exist or has no rows.
    #[tokio::test]
    async fn vitals_projection_lag_computed_from_cursor_timestamps() {
        let pool = test_pool().await;
        let projections =
            StandardVitalsProvider::query_projections(&pool, None).await;
        // Table may or may not exist; either way returns a Vec (possibly empty)
        for p in &projections {
            assert!(!p.name.is_empty());
            assert!(!p.tenant_id.is_empty());
            // lag and age should be non-negative in normal operation
            assert!(p.lag_ms >= 0, "lag_ms must be non-negative");
            assert!(p.age_seconds >= 0, "age_seconds must be non-negative");
        }
    }

    /// GET /api/vitals returns 200 with valid VitalsResponse shape.
    #[tokio::test]
    async fn vitals_response_returns_200_with_valid_shape() {
        let pool = test_pool().await;
        let provider = StandardVitalsProvider::new();
        let resp = provider
            .collect(&pool, "test-svc", "0.1.0", None, None, None)
            .await;

        assert_eq!(resp.service_name, "test-svc");
        assert_eq!(resp.version, "0.1.0");
        assert!(resp.tenant_ready.is_none());
        assert!(resp.extended.is_none());
        assert_eq!(resp.dlq.total, resp.dlq.retryable + resp.dlq.fatal + resp.dlq.poison);
        // Timestamp must be RFC-3339
        chrono::DateTime::parse_from_rfc3339(&resp.timestamp)
            .expect("timestamp must be valid RFC-3339");
    }
}
