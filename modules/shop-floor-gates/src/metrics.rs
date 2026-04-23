use axum::{extract::State, http::StatusCode};
use prometheus::{Encoder, IntCounter, IntCounterVec, Opts, Registry, TextEncoder};
use std::sync::Arc;

use crate::AppState;

pub struct SfgMetrics {
    pub holds_placed: IntCounter,
    pub holds_released: IntCounter,
    pub handoffs_initiated: IntCounter,
    pub handoffs_accepted: IntCounter,
    pub verifications_completed: IntCounter,
    pub signoffs_recorded: IntCounter,
    pub events_published: IntCounterVec,
    registry: Registry,
}

impl SfgMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let holds_placed =
            IntCounter::new("sfg_holds_placed_total", "Total traveler holds placed")?;
        registry.register(Box::new(holds_placed.clone()))?;

        let holds_released =
            IntCounter::new("sfg_holds_released_total", "Total traveler holds released")?;
        registry.register(Box::new(holds_released.clone()))?;

        let handoffs_initiated = IntCounter::new(
            "sfg_handoffs_initiated_total",
            "Total operation handoffs initiated",
        )?;
        registry.register(Box::new(handoffs_initiated.clone()))?;

        let handoffs_accepted = IntCounter::new(
            "sfg_handoffs_accepted_total",
            "Total operation handoffs accepted",
        )?;
        registry.register(Box::new(handoffs_accepted.clone()))?;

        let verifications_completed = IntCounter::new(
            "sfg_verifications_completed_total",
            "Total start verifications completed",
        )?;
        registry.register(Box::new(verifications_completed.clone()))?;

        let signoffs_recorded =
            IntCounter::new("sfg_signoffs_recorded_total", "Total signoffs recorded")?;
        registry.register(Box::new(signoffs_recorded.clone()))?;

        let events_published = IntCounterVec::new(
            Opts::new("sfg_events_published_total", "Events published by type"),
            &["event_type"],
        )?;
        registry.register(Box::new(events_published.clone()))?;

        Ok(Self {
            holds_placed,
            holds_released,
            handoffs_initiated,
            handoffs_accepted,
            verifications_completed,
            signoffs_recorded,
            events_published,
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
