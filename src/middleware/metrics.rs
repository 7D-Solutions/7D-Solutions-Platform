use axum::{
    body::Body,
    extract::State,
    http::Request,
    middleware::Next,
    response::Response,
};
use std::{sync::Arc, time::Instant};

use crate::metrics::Metrics;

#[derive(Clone)]
pub struct MetricsMiddlewareState {
    pub metrics: Metrics,
}

pub async fn metrics_middleware(
    State(state): State<Arc<MetricsMiddlewareState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let method = req.method().to_string();
    let start = Instant::now();

    let res = next.run(req).await;

    let status = res.status().as_u16().to_string();
    let elapsed = start.elapsed().as_secs_f64();

    state
        .metrics
        .http_request_duration_seconds
        .with_label_values(&[&path, &method, &status])
        .observe(elapsed);

    res
}
