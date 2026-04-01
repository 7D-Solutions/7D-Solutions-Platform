use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts};

#[derive(Clone)]
pub struct ProductionMetrics {
    pub production_operations_total: IntCounter,
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
}

impl ProductionMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let production_operations_total = IntCounter::new(
            "production_operations_total",
            "Total number of production operations processed",
        )?;
        prometheus::register(Box::new(production_operations_total.clone()))?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "production_http_request_duration_seconds",
                "HTTP request duration in seconds",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route", "status"],
        )?;
        prometheus::register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new("production_http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
        )?;
        prometheus::register(Box::new(http_requests_total.clone()))?;

        Ok(Self {
            production_operations_total,
            http_request_duration_seconds,
            http_requests_total,
        })
    }
}
