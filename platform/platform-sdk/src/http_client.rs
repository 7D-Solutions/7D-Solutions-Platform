//! Typed HTTP client for calling platform services with auto-injected headers and retry.

use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

use security::claims::VerifiedClaims;

/// HTTP client that injects platform headers and retries on 429/503.
///
/// ```rust,ignore
/// let party = PlatformClient::new(env::var("PARTY_BASE_URL")?);
/// let resp = party.get("/api/parties/123", &claims).await?;
/// ```
pub struct PlatformClient {
    client: Client,
    base_url: String,
    bearer_token: Option<String>,
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 100;

impl PlatformClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            bearer_token: None,
        }
    }

    /// Set a bearer token for the Authorization header (e.g. a service token).
    pub fn with_bearer_token(mut self, token: String) -> Self {
        self.bearer_token = Some(token);
        self
    }

    pub async fn get(&self, path: &str, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_with_retry(self.client.get(self.url(path)), claims).await
    }

    pub async fn post<T: Serialize>(&self, path: &str, body: &T, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_with_retry(self.client.post(self.url(path)).json(body), claims).await
    }

    pub async fn put<T: Serialize>(&self, path: &str, body: &T, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_with_retry(self.client.put(self.url(path)).json(body), claims).await
    }

    pub async fn delete(&self, path: &str, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_with_retry(self.client.delete(self.url(path)), claims).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn inject_headers(&self, mut req: reqwest::RequestBuilder, claims: &VerifiedClaims, correlation_id: &Uuid) -> reqwest::RequestBuilder {
        req = req
            .header("x-tenant-id", claims.tenant_id.to_string())
            .header("x-correlation-id", correlation_id.to_string());

        if let Some(app_id) = &claims.app_id {
            req = req.header("x-app-id", app_id.to_string());
        }

        if let Some(token) = &self.bearer_token {
            req = req.header("authorization", format!("Bearer {token}"));
        }

        req
    }

    async fn send_with_retry(
        &self,
        builder: reqwest::RequestBuilder,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        let correlation_id = Uuid::new_v4();
        let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);

        for attempt in 0..=MAX_RETRIES {
            let req = builder.try_clone().expect("request must be cloneable");
            let req = self.inject_headers(req, claims, &correlation_id);
            let resp = req.send().await?;

            let status = resp.status();
            if attempt < MAX_RETRIES && (status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE) {
                tokio::time::sleep(backoff).await;
                backoff *= 2;
                continue;
            }

            return Ok(resp);
        }

        unreachable!("loop always returns")
    }
}
