use crate::config::Config;
use http::HeaderName;
use tower_http::cors::CorsLayer;

/// Build CORS layer from configuration.
///
/// - Empty `cors_origins` → deny all cross-origin requests
/// - Explicit origins → allow only those origins
///
/// Always allows `If-Match` header for optimistic concurrency on document updates.
pub fn build_cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<http::HeaderValue> = config
        .cors_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods(tower_http::cors::Any)
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            HeaderName::from_static("if-match"),
            HeaderName::from_static("if-none-match"),
        ])
        .expose_headers([http::header::ETAG])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BusType;
    use axum::{routing::get, Router};
    use http::Request;
    use tower::ServiceExt;

    fn test_config(origins: Vec<String>) -> Config {
        Config {
            database_url: "postgres://test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8102,
            cors_origins: origins,
            env: "test".to_string(),
        }
    }

    type BoxErr = Box<dyn std::error::Error>;

    #[tokio::test]
    async fn cors_disallowed_origin_gets_no_headers() -> Result<(), BoxErr> {
        let config = test_config(vec!["https://allowed.example.com".to_string()]);
        let cors = build_cors_layer(&config);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(cors);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("origin", "https://evil.example.com")
                    .body(axum::body::Body::empty())?,
            )
            .await?;

        assert!(
            !resp.headers().contains_key("access-control-allow-origin"),
            "disallowed origin must not receive CORS allow header"
        );
        Ok(())
    }

    #[tokio::test]
    async fn cors_empty_origins_denies_all() -> Result<(), BoxErr> {
        let config = test_config(vec![]);
        let cors = build_cors_layer(&config);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(cors);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("origin", "https://any.example.com")
                    .body(axum::body::Body::empty())?,
            )
            .await?;

        assert!(
            !resp.headers().contains_key("access-control-allow-origin"),
            "empty allowlist must deny all origins"
        );
        Ok(())
    }

    #[tokio::test]
    async fn cors_allowed_origin_gets_headers() -> Result<(), BoxErr> {
        let config = test_config(vec!["https://allowed.example.com".to_string()]);
        let cors = build_cors_layer(&config);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(cors);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("origin", "https://allowed.example.com")
                    .body(axum::body::Body::empty())?,
            )
            .await?;

        assert_eq!(
            resp.headers()
                .get("access-control-allow-origin")
                .expect("allowed origin must receive CORS header"),
            "https://allowed.example.com"
        );
        Ok(())
    }

    #[tokio::test]
    async fn cors_preflight_disallowed_origin_no_headers() -> Result<(), BoxErr> {
        let config = test_config(vec!["https://allowed.example.com".to_string()]);
        let cors = build_cors_layer(&config);

        let app = Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(cors);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/test")
                    .header("origin", "https://evil.example.com")
                    .header("access-control-request-method", "POST")
                    .body(axum::body::Body::empty())?,
            )
            .await?;

        assert!(
            !resp.headers().contains_key("access-control-allow-origin"),
            "preflight for disallowed origin must not receive CORS headers"
        );
        Ok(())
    }
}
