//! Service-to-service authentication middleware
//!
//! This middleware verifies that incoming requests to operational endpoints
//! include a valid service authentication token in the Authorization header.

use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use security::verify_service_token;
use tracing::{debug, warn};

/// Service authentication middleware
///
/// Verifies the Authorization header contains a valid service token.
/// Rejects requests with 401 Unauthorized if:
/// - Authorization header is missing
/// - Token format is invalid
/// - Token signature is invalid
/// - Token is expired
///
/// # Example
///
/// ```ignore
/// use axum::{middleware, Router};
///
/// let app = Router::new()
///     .route("/api/ready", get(health::ready))
///     .route("/api/version", get(health::version))
///     .layer(middleware::from_fn(service_auth_middleware));
/// ```
pub async fn service_auth_middleware(
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // Extract Authorization header
    let auth_header = match headers.get("authorization") {
        Some(value) => value,
        None => {
            warn!("Missing Authorization header on protected endpoint");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "Missing Authorization header"
                })),
            )
                .into_response();
        }
    };

    // Parse Bearer token
    let auth_str = match auth_header.to_str() {
        Ok(s) => s,
        Err(_) => {
            warn!("Invalid Authorization header encoding");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "Invalid Authorization header encoding"
                })),
            )
                .into_response();
        }
    };

    // Extract token from "Bearer <token>" format
    let token = match auth_str.strip_prefix("Bearer ") {
        Some(t) => t,
        None => {
            warn!("Authorization header missing 'Bearer ' prefix");
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": "Authorization header must use Bearer scheme"
                })),
            )
                .into_response();
        }
    };

    // Verify token
    match verify_service_token(token) {
        Ok(claims) => {
            debug!("Authenticated service request from: {}", claims.service_name);
            next.run(request).await
        }
        Err(e) => {
            warn!("Invalid service token: {}", e);
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "unauthorized",
                    "message": format!("Invalid service token: {}", e)
                })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use security::generate_service_token;
    use std::env;

    #[tokio::test]
    async fn test_missing_auth_header() {
        env::set_var("SERVICE_AUTH_SECRET", "test-secret");

        let headers = HeaderMap::new();
        let request = Request::builder().body(Body::empty()).unwrap();
        let next = Next::new(|_req: Request| async { StatusCode::OK.into_response() });

        let response = service_auth_middleware(headers, request, next).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_invalid_bearer_format() {
        env::set_var("SERVICE_AUTH_SECRET", "test-secret");

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "InvalidFormat token123".parse().unwrap());

        let request = Request::builder().body(Body::empty()).unwrap();
        let next = Next::new(|_req: Request| async { StatusCode::OK.into_response() });

        let response = service_auth_middleware(headers, request, next).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_valid_token_passes() {
        env::set_var("SERVICE_AUTH_SECRET", "test-secret");

        let token = generate_service_token("test-service", None).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );

        let request = Request::builder().body(Body::empty()).unwrap();
        let next = Next::new(|_req: Request| async { StatusCode::OK.into_response() });

        let response = service_auth_middleware(headers, request, next).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
