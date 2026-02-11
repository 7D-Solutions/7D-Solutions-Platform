use axum::{
    body::Body,
    http::{HeaderMap, Request},
    middleware::Next,
    response::Response,
};
use std::net::SocketAddr;

#[derive(Clone, Debug)]
pub struct ClientMeta {
    pub ip: String,
    pub user_agent: Option<String>,
}

fn header_ip(headers: &HeaderMap) -> Option<String> {
    // Prefer X-Forwarded-For (first IP), then X-Real-IP
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = xff.split(',').next().map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(ip) = first {
            return Some(ip.to_string());
        }
    }
    if let Some(xri) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = xri.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }
    None
}

pub fn extract_ip(headers: &HeaderMap, connect: Option<SocketAddr>) -> String {
    header_ip(headers)
        .or_else(|| connect.map(|c| c.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

pub async fn client_meta_middleware(
    mut req: Request<Body>,
    next: Next,
) -> Response {
    // ConnectInfo is only available if server uses into_make_service_with_connect_info::<SocketAddr>()
    let connect = req.extensions().get::<SocketAddr>().cloned();

    let headers = req.headers();
    let ip = extract_ip(headers, connect);

    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    req.extensions_mut().insert(ClientMeta { ip, user_agent });

    next.run(req).await
}

pub fn get_client_meta(ext: &axum::http::Extensions) -> Option<ClientMeta> {
    ext.get::<ClientMeta>().cloned()
}
