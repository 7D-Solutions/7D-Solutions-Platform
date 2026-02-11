use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Request},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

pub const TRACE_ID_HEADER: &str = "x-trace-id";

pub async fn trace_id_middleware(mut req: Request<Body>, next: Next) -> Response {
    let trace_id = req
        .headers()
        .get(TRACE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    req.extensions_mut().insert(trace_id.clone());

    let mut res = next.run(req).await;
    let headers: &mut HeaderMap = res.headers_mut();
    headers.insert(
        TRACE_ID_HEADER,
        HeaderValue::from_str(&trace_id).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    res
}

pub fn get_trace_id_from_extensions(ext: &axum::http::Extensions) -> String {
    ext.get::<String>()
        .cloned()
        .unwrap_or_else(|| "missing-trace-id".to_string())
}
