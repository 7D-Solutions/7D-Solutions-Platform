use axum::{extract::State, http::StatusCode};
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};
use std::sync::Arc;

use crate::AppState;

pub struct SoMetrics {
    pub orders_created_total: IntCounter,
    pub orders_booked_total: IntCounter,
    pub orders_cancelled_total: IntCounter,
    pub blankets_activated_total: IntCounter,
    pub releases_created_total: IntCounter,
    pub outbox_queue_depth: IntGauge,
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
    registry: Registry,
}

impl SoMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let orders_created_total =
            IntCounter::new("so_orders_created_total", "Total sales orders created")?;
        registry.register(Box::new(orders_created_total.clone()))?;

        let orders_booked_total =
            IntCounter::new("so_orders_booked_total", "Total sales orders booked")?;
        registry.register(Box::new(orders_booked_total.clone()))?;

        let orders_cancelled_total =
            IntCounter::new("so_orders_cancelled_total", "Total sales orders cancelled")?;
        registry.register(Box::new(orders_cancelled_total.clone()))?;

        let blankets_activated_total = IntCounter::new(
            "so_blankets_activated_total",
            "Total blanket orders activated",
        )?;
        registry.register(Box::new(blankets_activated_total.clone()))?;

        let releases_created_total = IntCounter::new(
            "so_releases_created_total",
            "Total blanket releases created",
        )?;
        registry.register(Box::new(releases_created_total.clone()))?;

        let outbox_queue_depth =
            IntGauge::new("so_outbox_queue_depth", "Unpublished outbox events")?;
        registry.register(Box::new(outbox_queue_depth.clone()))?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "so_http_request_duration_seconds",
                "HTTP request latency in seconds",
            ),
            &["method", "path", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new("so_http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        Ok(Self {
            orders_created_total,
            orders_booked_total,
            orders_cancelled_total,
            blankets_activated_total,
            releases_created_total,
            outbox_queue_depth,
            http_request_duration_seconds,
            http_requests_total,
            registry,
        })
    }

    pub fn export(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&metric_families, &mut buf).unwrap_or(());
        String::from_utf8(buf).unwrap_or_default()
    }
}

pub async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        state.metrics.export(),
    )
}
