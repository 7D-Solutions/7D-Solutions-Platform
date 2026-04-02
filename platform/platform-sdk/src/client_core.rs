//! Shared helpers for generated typed clients.
//!
//! Every generated client calls [`parse_response`] or [`parse_empty`] — if
//! these have a bug, **all** clients break. Handle edge cases defensively.

use std::future::Future;

use platform_http_contracts::ApiError;
use reqwest::Response;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Errors returned by typed client methods.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Server returned a structured API error.
    #[error("API error (HTTP {status}): {error}")]
    Api { status: u16, error: ApiError },

    /// Server returned a non-JSON body (e.g. plain-text 502 from a proxy).
    #[error("unexpected response (HTTP {status}): {body}")]
    Unexpected { status: u16, body: String },

    /// 2xx response whose body could not be deserialized.
    #[error("failed to deserialize response: {0}")]
    Deserialize(#[source] serde_json::Error),

    /// Network / connection error.
    #[error("network error: {0}")]
    Network(#[source] reqwest::Error),

    /// Query-parameter encoding failed.
    #[error("query encoding error: {0}")]
    QueryEncode(#[source] serde_urlencoded::ser::Error),
}

impl ClientError {
    /// Check whether this error carries a specific HTTP status code.
    pub fn is_status(&self, code: u16) -> bool {
        match self {
            Self::Api { status, .. } | Self::Unexpected { status, .. } => *status == code,
            _ => false,
        }
    }

    /// Shorthand for `is_status(409)`.
    pub fn is_conflict(&self) -> bool {
        self.is_status(409)
    }

    /// Shorthand for `is_status(404)`.
    pub fn is_not_found(&self) -> bool {
        self.is_status(404)
    }
}

// ── Public helpers ────────────────────────────────────────────────────

/// Get-or-create: try `get` first; if it returns 404, call `create`.
///
/// ```rust,ignore
/// let party = ensure(
///     parties.get_party(&claims, party_id),
///     || parties.create_party(&claims, &body),
/// ).await?;
/// ```
pub async fn ensure<T, GetFut, CreateFn, CreateFut>(
    get: GetFut,
    create: CreateFn,
) -> Result<T, ClientError>
where
    GetFut: Future<Output = Result<T, ClientError>>,
    CreateFn: FnOnce() -> CreateFut,
    CreateFut: Future<Output = Result<T, ClientError>>,
{
    match get.await {
        Ok(val) => Ok(val),
        Err(e) if e.is_not_found() => create().await,
        Err(e) => Err(e),
    }
}

/// Deserialize a successful JSON response or return a typed error.
///
/// For 204 / empty-body responses use [`parse_empty`] instead — calling
/// this function on an empty body will produce [`ClientError::Deserialize`].
pub async fn parse_response<T: DeserializeOwned>(resp: Response) -> Result<T, ClientError> {
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(error_from_body(status, resp).await);
    }
    let bytes = resp.bytes().await.map_err(ClientError::Network)?;
    serde_json::from_slice(&bytes).map_err(ClientError::Deserialize)
}

/// Validate a response that carries no body (e.g. 204 No Content, 200 OK
/// on a DELETE).
pub async fn parse_empty(resp: Response) -> Result<(), ClientError> {
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(error_from_body(status, resp).await);
    }
    Ok(())
}

/// Build a path with URL-encoded query parameters.
///
/// ```rust,ignore
/// let url = build_query_url("/api/parties", &ListParams { page: 1, limit: 25 })?;
/// // => "/api/parties?page=1&limit=25"
/// ```
pub fn build_query_url<T: Serialize>(path: &str, params: &T) -> Result<String, ClientError> {
    let qs = serde_urlencoded::to_string(params).map_err(ClientError::QueryEncode)?;
    if qs.is_empty() {
        Ok(path.to_string())
    } else {
        Ok(format!("{path}?{qs}"))
    }
}

// ── Internals ─────────────────────────────────────────────────────────

/// Try to parse the body as an [`ApiError`]; fall back to [`ClientError::Unexpected`].
async fn error_from_body(status: u16, resp: Response) -> ClientError {
    let body = resp.text().await.unwrap_or_default();
    match serde_json::from_str::<ApiError>(&body) {
        Ok(api_err) => ClientError::Api {
            status,
            error: api_err,
        },
        Err(_) => ClientError::Unexpected { status, body },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_query_url ───────────────────────────────────────────────

    #[derive(Serialize)]
    struct Params {
        page: u32,
        limit: u32,
    }

    type BoxErr = Box<dyn std::error::Error>;

    #[test]
    fn query_url_appends_params() -> Result<(), BoxErr> {
        let url = build_query_url("/api/items", &Params { page: 2, limit: 50 })?;
        assert_eq!(url, "/api/items?page=2&limit=50");
        Ok(())
    }

    #[derive(Serialize)]
    struct Empty {}

    #[test]
    fn query_url_no_params_returns_bare_path() -> Result<(), BoxErr> {
        let url = build_query_url("/api/items", &Empty {})?;
        assert_eq!(url, "/api/items");
        Ok(())
    }

    #[derive(Serialize)]
    struct OptionalParams {
        #[serde(skip_serializing_if = "Option::is_none")]
        search: Option<String>,
        page: u32,
    }

    #[test]
    fn query_url_skips_none_fields() -> Result<(), BoxErr> {
        let url = build_query_url(
            "/api/items",
            &OptionalParams {
                search: None,
                page: 1,
            },
        )?;
        assert_eq!(url, "/api/items?page=1");
        Ok(())
    }

    #[test]
    fn query_url_includes_some_fields() -> Result<(), BoxErr> {
        let url = build_query_url(
            "/api/items",
            &OptionalParams {
                search: Some("bolt".into()),
                page: 3,
            },
        )?;
        assert_eq!(url, "/api/items?search=bolt&page=3");
        Ok(())
    }

    // ── parse_response / parse_empty (integration with real HTTP) ─────

    fn make_response(status: u16, body: impl Into<String>) -> Response {
        let resp = http::Response::builder()
            .status(status)
            .body(body.into())
            .expect("test response builder");
        Response::from(resp)
    }

    #[tokio::test]
    async fn parse_response_deserializes_json() -> Result<(), BoxErr> {
        let body = serde_json::json!({"id": 1, "name": "test"});
        let resp = make_response(200, body.to_string());
        let val: serde_json::Value = parse_response(resp).await?;
        assert_eq!(val["name"], "test");
        Ok(())
    }

    #[tokio::test]
    async fn parse_response_on_404_returns_api_error() {
        let api_err = serde_json::json!({
            "error": "not_found",
            "message": "Item 42 not found"
        });
        let resp = make_response(404, api_err.to_string());
        let err = parse_response::<serde_json::Value>(resp).await.unwrap_err();
        match err {
            ClientError::Api { status, error } => {
                assert_eq!(status, 404);
                assert_eq!(error.error, "not_found");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parse_response_on_502_plain_text_returns_unexpected() {
        let resp = make_response(502, "<html>Bad Gateway</html>");
        let err = parse_response::<serde_json::Value>(resp).await.unwrap_err();
        match err {
            ClientError::Unexpected { status, body } => {
                assert_eq!(status, 502);
                assert!(body.contains("Bad Gateway"));
            }
            other => panic!("expected Unexpected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parse_empty_on_204_succeeds() -> Result<(), BoxErr> {
        let resp = make_response(204, "");
        parse_empty(resp).await?;
        Ok(())
    }

    #[tokio::test]
    async fn parse_empty_on_error_returns_error() {
        let api_err = serde_json::json!({
            "error": "forbidden",
            "message": "not allowed"
        });
        let resp = make_response(403, api_err.to_string());
        let err = parse_empty(resp).await.unwrap_err();
        match err {
            ClientError::Api { status, error } => {
                assert_eq!(status, 403);
                assert_eq!(error.error, "forbidden");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parse_response_on_bad_json_returns_deserialize_error() {
        let resp = make_response(200, "not json at all");
        let err = parse_response::<serde_json::Value>(resp).await.unwrap_err();
        assert!(matches!(err, ClientError::Deserialize(_)));
    }

    // ── is_status / is_conflict / is_not_found ──────────────────────────

    #[test]
    fn is_status_matches_api_error() {
        let err = ClientError::Api {
            status: 409,
            error: ApiError::new(409, "conflict", "duplicate"),
        };
        assert!(err.is_status(409));
        assert!(err.is_conflict());
        assert!(!err.is_not_found());
        assert!(!err.is_status(500));
    }

    #[test]
    fn is_status_matches_unexpected_error() {
        let err = ClientError::Unexpected {
            status: 404,
            body: "not found".into(),
        };
        assert!(err.is_status(404));
        assert!(err.is_not_found());
        assert!(!err.is_conflict());
    }

    // ── ensure ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ensure_returns_get_result_when_found() {
        let result = ensure(
            async { Ok::<_, ClientError>(42u32) },
            || async { Ok(99u32) },
        )
        .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn ensure_creates_on_404() {
        let result = ensure(
            async {
                Err::<u32, _>(ClientError::Unexpected {
                    status: 404,
                    body: "not found".into(),
                })
            },
            || async { Ok(99u32) },
        )
        .await;
        assert_eq!(result.unwrap(), 99);
    }

    #[tokio::test]
    async fn ensure_propagates_non_404_errors() {
        let result = ensure(
            async {
                Err::<u32, _>(ClientError::Unexpected {
                    status: 500,
                    body: "server error".into(),
                })
            },
            || async { Ok(99u32) },
        )
        .await;
        assert!(result.unwrap_err().is_status(500));
    }

    #[test]
    fn is_status_returns_false_for_deserialize_errors() {
        let err = ClientError::Deserialize(
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
        );
        assert!(!err.is_status(500));
        assert!(!err.is_conflict());
        assert!(!err.is_not_found());
    }
}
