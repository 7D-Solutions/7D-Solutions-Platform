//! Metrics collection for the stabilization gate harness.
//!
//! Records latency histograms (p50/p95/p99), operation counts, error counts,
//! throughput rates, and wall-clock durations for each benchmark scenario.

use std::time::{Duration, Instant};

/// Collects raw latency samples and error counts for a single scenario.
#[derive(Debug, Clone, Default)]
pub struct MetricsSamples {
    /// Individual operation latencies in milliseconds.
    pub latencies_ms: Vec<f64>,
    /// Total successful operations recorded.
    pub total_ops: u64,
    /// Total errors encountered.
    pub errors: u64,
    /// Wall-clock duration of the entire scenario in milliseconds.
    pub wall_clock_ms: f64,
}

impl MetricsSamples {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one successful operation with its latency.
    pub fn record_latency(&mut self, d: Duration) {
        self.latencies_ms.push(d.as_secs_f64() * 1000.0);
        self.total_ops += 1;
    }

    /// Record one error (no latency recorded for failed ops).
    pub fn record_error(&mut self) {
        self.errors += 1;
    }

    /// Set the overall wall-clock duration for the scenario.
    pub fn set_wall_clock(&mut self, d: Duration) {
        self.wall_clock_ms = d.as_secs_f64() * 1000.0;
    }

    /// 50th-percentile latency in milliseconds.
    pub fn p50(&self) -> f64 {
        percentile(&self.latencies_ms, 50.0)
    }

    /// 95th-percentile latency in milliseconds.
    pub fn p95(&self) -> f64 {
        percentile(&self.latencies_ms, 95.0)
    }

    /// 99th-percentile latency in milliseconds.
    pub fn p99(&self) -> f64 {
        percentile(&self.latencies_ms, 99.0)
    }

    /// Operations per second computed from wall-clock time.
    pub fn throughput_ops_per_sec(&self) -> f64 {
        if self.wall_clock_ms == 0.0 {
            return 0.0;
        }
        (self.total_ops as f64) / (self.wall_clock_ms / 1000.0)
    }

    /// Serialize to a JSON value suitable for embedding in reports.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "p50_ms": self.p50(),
            "p95_ms": self.p95(),
            "p99_ms": self.p99(),
            "total_ops": self.total_ops,
            "errors": self.errors,
            "wall_clock_ms": self.wall_clock_ms,
            "throughput_ops_per_sec": self.throughput_ops_per_sec(),
        })
    }
}

/// Compute the given percentile from a slice of f64 values.
fn percentile(samples: &[f64], pct: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((pct / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Simple wall-clock timer for measuring scenario durations.
pub struct Timer {
    start: Instant,
}

impl Timer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_single_sample() {
        let mut m = MetricsSamples::new();
        m.record_latency(Duration::from_millis(42));
        assert_eq!(m.p50(), 42.0);
        assert_eq!(m.p99(), 42.0);
    }

    #[test]
    fn percentile_ordered() {
        let mut m = MetricsSamples::new();
        for i in 1..=100u64 {
            m.record_latency(Duration::from_millis(i));
        }
        assert!((m.p50() - 50.0).abs() < 2.0);
        assert!((m.p95() - 95.0).abs() < 2.0);
        assert!((m.p99() - 99.0).abs() < 2.0);
    }
}
