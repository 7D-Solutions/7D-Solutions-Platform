//! Timing and gauge-snapshot helpers for the ingestion pipeline.
//!
//! Provides:
//! - [`IngestTimer`]: wall-clock timer used to measure per-event processing
//!   latency, which is then fed into `ReportingMetrics::record_ingestion_lag`.
//! - [`checkpoint_stats`]: DB query that summarises the current ingestion
//!   checkpoint table for gauge-refresh use by `metrics::refresh_gauges`.

use sqlx::PgPool;
use std::time::Instant;

// ── IngestTimer ───────────────────────────────────────────────────────────────

/// Measures wall-clock time for a single ingestion operation.
///
/// # Usage
///
/// ```rust,ignore
/// let timer = IngestTimer::start();
/// handler.handle(...).await?;
/// metrics.record_ingestion_lag("ar.events.ar.ar_aging_updated", timer.elapsed_secs());
/// ```
pub struct IngestTimer {
    started_at: Instant,
}

impl IngestTimer {
    /// Start a new timer.
    pub fn start() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }

    /// Return elapsed time in seconds since [`IngestTimer::start`].
    pub fn elapsed_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }
}

// ── Checkpoint stats ──────────────────────────────────────────────────────────

/// Aggregate checkpoint statistics for the ingestion pipeline.
pub struct CheckpointStats {
    /// Total number of `(consumer_name, tenant_id)` checkpoint rows.
    pub total_checkpoints: i64,
    /// Number of distinct consumer names being tracked.
    pub distinct_consumers: i64,
}

/// Query the reporting DB for current ingestion checkpoint statistics.
///
/// Used by `metrics::refresh_gauges` to populate the
/// `reporting_ingestion_checkpoints` gauge.
pub async fn checkpoint_stats(pool: &PgPool) -> Result<CheckpointStats, sqlx::Error> {
    let total_checkpoints: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM rpt_ingestion_checkpoints")
            .fetch_one(pool)
            .await?;

    let distinct_consumers: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT consumer_name) FROM rpt_ingestion_checkpoints")
            .fetch_one(pool)
            .await?;

    Ok(CheckpointStats {
        total_checkpoints,
        distinct_consumers,
    })
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn ingest_timer_measures_elapsed() {
        let timer = IngestTimer::start();
        sleep(Duration::from_millis(15));
        let elapsed = timer.elapsed_secs();
        assert!(elapsed >= 0.010, "elapsed={elapsed} should be >= 10 ms");
        assert!(
            elapsed < 1.0,
            "elapsed={elapsed} should not be unreasonably large"
        );
    }

    #[test]
    fn ingest_timer_sequential_timers() {
        let t1 = IngestTimer::start();
        // Small gap so t1 is strictly older
        sleep(Duration::from_millis(5));
        let t2 = IngestTimer::start();
        let e1 = t1.elapsed_secs();
        let e2 = t2.elapsed_secs();
        assert!(e1 >= e2, "t1 started before t2, so e1 ({e1}) >= e2 ({e2})");
    }
}
