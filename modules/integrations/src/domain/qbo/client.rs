//! QBO REST API client implementation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use super::{classify_error, parse_api_error, QboApiAction, QboError, TokenProvider};

// ============================================================================
// Invoice creation types
// ============================================================================

/// A single line item on a QBO invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboLineItem {
    /// Amount in dollars (not cents).
    pub amount: f64,
    pub description: Option<String>,
    /// QBO Item.Id (e.g. "1" = Services). Optional — omit for untracked line items.
    pub item_ref: Option<String>,
}

/// Payload for creating a QBO invoice via POST /v3/company/{realm}/invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboInvoicePayload {
    /// QBO Customer.Id.
    pub customer_ref: String,
    pub line_items: Vec<QboLineItem>,
    /// Due date in YYYY-MM-DD format.
    pub due_date: Option<String>,
    /// AR invoice ID placed in QBO DocNumber for cross-reference.
    pub doc_number: Option<String>,
}

impl QboInvoicePayload {
    /// Serialize to QBO REST API wire format.
    pub(crate) fn to_qbo_json(&self) -> Value {
        let lines: Vec<Value> = self
            .line_items
            .iter()
            .map(|item| {
                let mut line = serde_json::json!({
                    "Amount": item.amount,
                    "DetailType": "SalesItemLineDetail",
                    "SalesItemLineDetail": {}
                });
                if let Some(ref ir) = item.item_ref {
                    line["SalesItemLineDetail"]["ItemRef"] = serde_json::json!({"value": ir});
                }
                if let Some(ref desc) = item.description {
                    line["Description"] = Value::String(desc.clone());
                }
                line
            })
            .collect();

        let mut body = serde_json::json!({
            "CustomerRef": {"value": &self.customer_ref},
            "Line": lines,
        });
        if let Some(ref dd) = self.due_date {
            body["DueDate"] = Value::String(dd.clone());
        }
        if let Some(ref dn) = self.doc_number {
            body["DocNumber"] = Value::String(dn.clone());
        }
        body
    }
}

/// Minor version appended to all QBO API requests.
pub const MINOR_VERSION: u32 = 75;
/// Max results per query page.
pub const MAX_RESULTS: u32 = 1000;
/// Max SyncToken retry attempts before giving up.
pub const SYNC_TOKEN_MAX_RETRIES: u32 = 3;

/// Async client for the QuickBooks Online REST API.
pub struct QboClient {
    http: reqwest::Client,
    base_url: String,
    realm_id: String,
    minor_version: u32,
    tokens: Arc<dyn TokenProvider>,
}

impl QboClient {
    pub fn new(base_url: &str, realm_id: &str, tokens: Arc<dyn TokenProvider>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            realm_id: realm_id.to_string(),
            minor_version: MINOR_VERSION,
            tokens,
        }
    }

    fn company_url(&self) -> String {
        format!("{}/company/{}", self.base_url, self.realm_id)
    }

    /// Build a read URL. Appends `?minorversion=N`.
    pub(crate) fn read_url(&self, path: &str) -> String {
        format!(
            "{}/{}?minorversion={}",
            self.company_url(),
            path,
            self.minor_version
        )
    }

    /// Build a write URL. Appends `?minorversion=N&requestid=UUID`.
    pub(crate) fn write_url(&self, path: &str) -> String {
        format!(
            "{}/{}?minorversion={}&requestid={}",
            self.company_url(),
            path,
            self.minor_version,
            Uuid::new_v4()
        )
    }

    /// Build the query endpoint URL.
    pub(crate) fn query_url(&self) -> String {
        format!(
            "{}/query?minorversion={}",
            self.company_url(),
            self.minor_version
        )
    }

    /// Build a CDC endpoint URL.
    ///
    /// Uses `Z` suffix (not `+00:00`) to avoid URL encoding issues with `+`.
    pub(crate) fn cdc_url(&self, entities: &[&str], changed_since: &DateTime<Utc>) -> String {
        format!(
            "{}/cdc?entities={}&changedSince={}&minorversion={}",
            self.company_url(),
            entities.join(","),
            changed_since.format("%Y-%m-%dT%H:%M:%SZ"),
            self.minor_version
        )
    }

    /// Build a paginated query string with STARTPOSITION and MAXRESULTS.
    pub fn paginated_query(base: &str, start: u32, max: u32) -> String {
        format!("{} STARTPOSITION {} MAXRESULTS {}", base, start, max)
    }

    // ========================================================================
    // Public API
    // ========================================================================

    /// GET a single entity by type and ID.
    pub async fn get_entity(&self, entity_type: &str, id: &str) -> Result<Value, QboError> {
        let url = self.read_url(&format!("{}/{}", entity_type.to_lowercase(), id));
        self.get_with_refresh(&url).await
    }

    /// Execute a raw QBO query statement.
    pub async fn query(&self, statement: &str) -> Result<Value, QboError> {
        let url = self.query_url();
        let token = self.tokens.get_token().await?;

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .header("Content-Type", "application/text")
            .body(statement.to_string())
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;

        if status == 200 {
            return parse_json(&body);
        }

        match classify_error(status, &body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                self.post_text(&url, statement, &new_token).await
            }
            QboApiAction::Backoff => Err(QboError::RateLimited),
            _ => Err(parse_api_error(&body)),
        }
    }

    /// Query all pages of results for a given base query.
    ///
    /// `entity_key` is the JSON key in QueryResponse (e.g., `"Invoice"`).
    pub async fn query_all(
        &self,
        base_query: &str,
        entity_key: &str,
    ) -> Result<Vec<Value>, QboError> {
        let mut results = Vec::new();
        let mut start = 1u32;

        loop {
            let stmt = Self::paginated_query(base_query, start, MAX_RESULTS);
            let response = self.query(&stmt).await?;
            let entities = response["QueryResponse"][entity_key]
                .as_array()
                .cloned()
                .unwrap_or_default();

            let count = entities.len() as u32;
            results.extend(entities);

            if count < MAX_RESULTS {
                break;
            }
            start += count;
        }

        Ok(results)
    }

    /// Update an entity with SyncToken optimistic locking and retry.
    ///
    /// On SyncToken stale (QBO error 5010), re-fetches the entity to get a
    /// fresh SyncToken and retries, up to [`SYNC_TOKEN_MAX_RETRIES`] times.
    pub async fn update_entity(
        &self,
        entity_type: &str,
        mut body: Value,
    ) -> Result<Value, QboError> {
        let entity_id = body["Id"]
            .as_str()
            .ok_or_else(|| QboError::Deserialize("update body missing Id".into()))?
            .to_string();

        for attempt in 0..=SYNC_TOKEN_MAX_RETRIES {
            let url = self.write_url(&entity_type.to_lowercase());
            let token = self.tokens.get_token().await?;

            let resp = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .header("Accept", "application/json")
                .json(&body)
                .send()
                .await?;

            let status = resp.status().as_u16();
            let resp_body = resp.text().await?;

            if status == 200 {
                return parse_json(&resp_body);
            }

            match classify_error(status, &resp_body) {
                QboApiAction::RetryWithFreshSyncToken if attempt < SYNC_TOKEN_MAX_RETRIES => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        max = SYNC_TOKEN_MAX_RETRIES,
                        entity_type,
                        entity_id = %entity_id,
                        "SyncToken stale — re-fetching"
                    );
                    let fresh = self.get_entity(entity_type, &entity_id).await?;
                    let key = capitalize(entity_type);
                    if let Some(st) = fresh[&key]["SyncToken"].as_str() {
                        body["SyncToken"] = Value::String(st.to_string());
                    }
                    continue;
                }
                QboApiAction::RetryWithFreshSyncToken => {
                    return Err(QboError::SyncTokenExhausted(SYNC_TOKEN_MAX_RETRIES));
                }
                QboApiAction::RefreshToken => {
                    let new_token = self.tokens.refresh_token().await?;
                    let url = self.write_url(&entity_type.to_lowercase());
                    return self.post_json(&url, &body, &new_token).await;
                }
                QboApiAction::Backoff => return Err(QboError::RateLimited),
                QboApiAction::Fail => return Err(parse_api_error(&resp_body)),
            }
        }

        Err(QboError::SyncTokenExhausted(SYNC_TOKEN_MAX_RETRIES))
    }

    /// Create a new invoice in QBO.
    ///
    /// Uses `write_url()` which appends `?requestid=UUID` for idempotency.
    /// Returns the `Invoice` object from the QBO response.
    pub async fn create_invoice(&self, payload: &QboInvoicePayload) -> Result<Value, QboError> {
        let url = self.write_url("invoice");
        let body = payload.to_qbo_json();
        let token = self.tokens.get_token().await?;

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Invoice"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                // Generate a fresh write_url so requestid changes on retry
                let retry_url = self.write_url("invoice");
                let val = self.post_json(&retry_url, &body, &new_token).await?;
                Ok(val["Invoice"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited),
            _ => Err(parse_api_error(&resp_body)),
        }
    }

    /// Call the CDC endpoint.
    pub async fn cdc(
        &self,
        entities: &[&str],
        changed_since: &DateTime<Utc>,
    ) -> Result<Value, QboError> {
        let url = self.cdc_url(entities, changed_since);
        self.get_with_refresh(&url).await
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    async fn get_with_refresh(&self, url: &str) -> Result<Value, QboError> {
        let token = self.tokens.get_token().await?;
        let resp = self
            .http
            .get(url)
            .bearer_auth(&token)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;

        if status == 200 {
            return parse_json(&body);
        }

        match classify_error(status, &body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let resp = self
                    .http
                    .get(url)
                    .bearer_auth(&new_token)
                    .header("Accept", "application/json")
                    .send()
                    .await?;
                let status = resp.status().as_u16();
                let body = resp.text().await?;
                if status == 200 {
                    parse_json(&body)
                } else {
                    Err(parse_api_error(&body))
                }
            }
            QboApiAction::Backoff => Err(QboError::RateLimited),
            _ => Err(parse_api_error(&body)),
        }
    }

    async fn post_text(&self, url: &str, text: &str, token: &str) -> Result<Value, QboError> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("Accept", "application/json")
            .header("Content-Type", "application/text")
            .body(text.to_string())
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if status == 200 {
            parse_json(&body)
        } else {
            Err(parse_api_error(&body))
        }
    }

    async fn post_json(&self, url: &str, json: &Value, token: &str) -> Result<Value, QboError> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("Accept", "application/json")
            .json(json)
            .send()
            .await?;

        let status = resp.status().as_u16();
        let body = resp.text().await?;
        if status == 200 {
            parse_json(&body)
        } else {
            Err(parse_api_error(&body))
        }
    }
}

fn parse_json(body: &str) -> Result<Value, QboError> {
    serde_json::from_str(body).map_err(|e| QboError::Deserialize(e.to_string()))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct FixedTokenProvider;

    #[async_trait::async_trait]
    impl TokenProvider for FixedTokenProvider {
        async fn get_token(&self) -> Result<String, QboError> {
            Ok("test-token".into())
        }
        async fn refresh_token(&self) -> Result<String, QboError> {
            Ok("test-token".into())
        }
    }

    fn test_client(base_url: &str) -> QboClient {
        QboClient::new(base_url, "12345", Arc::new(FixedTokenProvider))
    }

    #[test]
    fn read_url_appends_minorversion() {
        let c = test_client("https://sandbox-quickbooks.api.intuit.com/v3");
        let url = c.read_url("invoice/42");
        assert!(url.contains("minorversion=75"));
        assert!(url
            .starts_with("https://sandbox-quickbooks.api.intuit.com/v3/company/12345/invoice/42"));
    }

    #[test]
    fn write_url_has_minorversion_and_requestid() {
        let c = test_client("https://sandbox-quickbooks.api.intuit.com/v3");
        let url = c.write_url("invoice");
        assert!(url.contains("minorversion=75"), "missing minorversion");
        assert!(url.contains("requestid="), "missing requestid");
        let rid = url.split("requestid=").nth(1).expect("requestid param");
        assert_eq!(rid.len(), 36, "requestid must be UUID: {}", rid);
    }

    #[test]
    fn query_url_has_minorversion() {
        let c = test_client("https://example.com/v3");
        assert!(c.query_url().ends_with("query?minorversion=75"));
    }

    #[test]
    fn paginated_query_format() {
        let q = QboClient::paginated_query("SELECT * FROM Invoice WHERE Balance > '0'", 1, 1000);
        assert_eq!(
            q,
            "SELECT * FROM Invoice WHERE Balance > '0' STARTPOSITION 1 MAXRESULTS 1000"
        );
    }

    #[test]
    fn paginated_query_increments_start() {
        let q1 = QboClient::paginated_query("SELECT * FROM Customer", 1, 1000);
        let q2 = QboClient::paginated_query("SELECT * FROM Customer", 1001, 1000);
        assert!(q1.contains("STARTPOSITION 1 MAXRESULTS"));
        assert!(q2.contains("STARTPOSITION 1001 MAXRESULTS"));
    }

    #[test]
    fn sync_token_max_retries_is_three() {
        assert_eq!(SYNC_TOKEN_MAX_RETRIES, 3);
    }

    #[test]
    fn cdc_url_format() {
        let c = test_client("https://sandbox-quickbooks.api.intuit.com/v3");
        let since = Utc::now() - chrono::Duration::hours(1);
        let url = c.cdc_url(&["Customer", "Invoice"], &since);
        assert!(url.contains("entities=Customer,Invoice"));
        assert!(url.contains("changedSince="));
        assert!(url.contains("minorversion=75"));
    }

    // -- SyncToken retry tests using local axum server --

    struct SyncTestState {
        get_count: AtomicU32,
        post_count: AtomicU32,
        max_failures: u32,
    }

    async fn mock_get(
        axum::extract::State(s): axum::extract::State<Arc<SyncTestState>>,
    ) -> (axum::http::StatusCode, String) {
        s.get_count.fetch_add(1, Ordering::SeqCst);
        (
            axum::http::StatusCode::OK,
            r#"{"Invoice":{"Id":"1","SyncToken":"99"}}"#.into(),
        )
    }

    async fn mock_post(
        axum::extract::State(s): axum::extract::State<Arc<SyncTestState>>,
    ) -> (axum::http::StatusCode, String) {
        let n = s.post_count.fetch_add(1, Ordering::SeqCst);
        if n < s.max_failures {
            (
                axum::http::StatusCode::BAD_REQUEST,
                r#"{"Fault":{"Error":[{"Message":"Stale Object Error","Detail":"SyncToken mismatch","code":"5010"}],"type":"ValidationFault"}}"#.into(),
            )
        } else {
            (
                axum::http::StatusCode::OK,
                r#"{"Invoice":{"Id":"1","SyncToken":"100"}}"#.into(),
            )
        }
    }

    async fn start_server(max_failures: u32) -> (String, Arc<SyncTestState>) {
        let state = Arc::new(SyncTestState {
            get_count: AtomicU32::new(0),
            post_count: AtomicU32::new(0),
            max_failures,
        });
        let app = axum::Router::new()
            .route(
                "/v3/company/{realm}/invoice/{id}",
                axum::routing::get(mock_get),
            )
            .route(
                "/v3/company/{realm}/invoice",
                axum::routing::post(mock_post),
            )
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server crashed")
        });
        (format!("http://{}/v3", addr), state)
    }

    #[tokio::test]
    async fn sync_token_retry_succeeds_after_stale_errors() {
        let (base_url, state) = start_server(3).await;
        let client = test_client(&base_url);
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true});

        let result = client.update_entity("Invoice", body).await;
        assert!(result.is_ok(), "expected success: {:?}", result);
        assert_eq!(state.get_count.load(Ordering::SeqCst), 3, "3 re-fetches");
        assert_eq!(
            state.post_count.load(Ordering::SeqCst),
            4,
            "4 POST attempts (initial + 3 retries)"
        );
    }

    #[tokio::test]
    async fn sync_token_exhausted_after_max_retries() {
        let (base_url, state) = start_server(100).await;
        let client = test_client(&base_url);
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true});

        let result = client.update_entity("Invoice", body).await;
        assert!(
            matches!(result, Err(QboError::SyncTokenExhausted(3))),
            "expected SyncTokenExhausted: {:?}",
            result
        );
        assert_eq!(
            state.post_count.load(Ordering::SeqCst),
            4,
            "should attempt 4 POSTs then give up"
        );
    }

    // -- create_invoice tests --

    async fn start_create_server() -> (String, Arc<AtomicU32>) {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let app = axum::Router::new()
            .route(
                "/v3/company/{realm}/invoice",
                axum::routing::post(move || {
                    let c = count.clone();
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        (
                            axum::http::StatusCode::OK,
                            r#"{"Invoice":{"Id":"42","SyncToken":"0","DocNumber":"INV-001"}}"#,
                        )
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server crashed")
        });
        (format!("http://{}/v3", addr), call_count)
    }

    #[tokio::test]
    async fn create_invoice_returns_invoice_id() {
        let (base_url, call_count) = start_create_server().await;
        let client = test_client(&base_url);

        let payload = QboInvoicePayload {
            customer_ref: "1".to_string(),
            line_items: vec![QboLineItem {
                amount: 150.00,
                description: Some("Service fee".to_string()),
                item_ref: None,
            }],
            due_date: Some("2026-05-01".to_string()),
            doc_number: Some("INV-001".to_string()),
        };

        let result = client.create_invoice(&payload).await.expect("create_invoice failed");
        assert_eq!(result["Id"].as_str(), Some("42"), "returned invoice must have Id=42");
        assert_eq!(call_count.load(Ordering::SeqCst), 1, "one POST to QBO");
    }

    #[tokio::test]
    async fn qbo_invoice_payload_serializes_correctly() {
        let payload = QboInvoicePayload {
            customer_ref: "5".to_string(),
            line_items: vec![QboLineItem {
                amount: 500.00,
                description: Some("Consulting".to_string()),
                item_ref: Some("1".to_string()),
            }],
            due_date: Some("2026-06-15".to_string()),
            doc_number: Some("DOC-999".to_string()),
        };
        let json = payload.to_qbo_json();
        assert_eq!(json["CustomerRef"]["value"].as_str(), Some("5"));
        assert_eq!(json["DueDate"].as_str(), Some("2026-06-15"));
        assert_eq!(json["DocNumber"].as_str(), Some("DOC-999"));
        let lines = json["Line"].as_array().expect("Line must be array");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["Amount"].as_f64(), Some(500.00));
        assert_eq!(lines[0]["SalesItemLineDetail"]["ItemRef"]["value"].as_str(), Some("1"));
    }
}
