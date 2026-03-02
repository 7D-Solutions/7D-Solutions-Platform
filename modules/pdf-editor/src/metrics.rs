//! Prometheus metrics for the PDF Editor module.
//!
//! SLO metrics exposed:
//! - `pdf_editor_http_request_duration_seconds{method, route, status}`: request latency
//! - `pdf_editor_http_requests_total{method, route, status}`: request count / error rate
//!
//! No PII in labels — method, route, status are operational values only.

use axum::{http::StatusCode, Json};
use lazy_static::lazy_static;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder,
};

use crate::routes::ErrorBody;

lazy_static! {
    pub static ref METRICS_REGISTRY: Registry = {
        let registry = Registry::new();
        registry
            .register(Box::new(HTTP_REQUEST_DURATION_SECONDS.clone()))
            .expect("register pdf_editor_http_request_duration_seconds");
        registry
            .register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
            .expect("register pdf_editor_http_requests_total");
        registry
    };
    pub static ref HTTP_REQUEST_DURATION_SECONDS: HistogramVec = HistogramVec::new(
        HistogramOpts::new(
            "pdf_editor_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0
        ]),
        &["method", "route", "status"],
    )
    .expect("create pdf_editor_http_request_duration_seconds");
    pub static ref HTTP_REQUESTS_TOTAL: IntCounterVec = IntCounterVec::new(
        Opts::new("pdf_editor_http_requests_total", "Total HTTP requests"),
        &["method", "route", "status"],
    )
    .expect("create pdf_editor_http_requests_total");
}

/// Record an HTTP request for SLO tracking.
pub fn record_http_request(method: &str, route: &str, status: &str, duration_secs: f64) {
    HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&[method, route, status])
        .observe(duration_secs);
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, route, status])
        .inc();
}

/// Axum handler for GET /metrics — renders Prometheus text format.
pub async fn metrics_handler() -> Result<String, (StatusCode, Json<ErrorBody>)> {
    let encoder = TextEncoder::new();
    let families = METRICS_REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&families, &mut buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new(
                "internal_error",
                &format!("Failed to encode metrics: {}", e),
            )),
        )
    })?;
    String::from_utf8(buffer).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new(
                "internal_error",
                &format!("Failed to convert metrics to UTF-8: {}", e),
            )),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_exports_request_latency() {
        record_http_request("GET", "/api/ready", "200", 0.002);
        let families = METRICS_REGISTRY.gather();
        let names: Vec<_> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names
                .iter()
                .any(|n| n.contains("http_request_duration_seconds")),
            "request latency histogram missing: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("http_requests_total")),
            "request count counter missing: {:?}",
            names
        );
    }
}
