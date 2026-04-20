//! QBO REST API client implementation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
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
    /// ISO-4217 currency code. Required for multi-currency realms; omit for
    /// single-currency companies (defaults to realm currency).
    pub currency_ref: Option<String>,
}

impl QboInvoicePayload {
    /// Serialize to QBO REST API wire format.
    ///
    /// Line-item amounts are forwarded as f64 without truncation to preserve
    /// precision specified by the caller.
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
        if let Some(ref currency) = self.currency_ref {
            body["CurrencyRef"] = serde_json::json!({"value": currency});
        }
        body
    }
}

// ============================================================================
// Customer creation types
// ============================================================================

/// Payload for creating a QBO customer via POST /v3/company/{realm}/customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboCustomerPayload {
    pub display_name: String,
    pub email: Option<String>,
    pub company_name: Option<String>,
    /// ISO-4217 currency code. Defaults to realm currency when absent.
    pub currency_ref: Option<String>,
}

impl QboCustomerPayload {
    pub(crate) fn to_qbo_json(&self) -> Value {
        let mut body = serde_json::json!({"DisplayName": &self.display_name});
        if let Some(ref email) = self.email {
            body["PrimaryEmailAddr"] = serde_json::json!({"Address": email});
        }
        if let Some(ref company) = self.company_name {
            body["CompanyName"] = Value::String(company.clone());
        }
        if let Some(ref currency) = self.currency_ref {
            body["CurrencyRef"] = serde_json::json!({"value": currency});
        }
        body
    }
}

// ============================================================================
// Payment creation types
// ============================================================================

/// Links a payment amount to a specific invoice for allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentLineApplication {
    /// QBO Invoice TxnId.
    pub invoice_id: String,
    pub amount: f64,
}

/// Payload for creating a QBO payment via POST /v3/company/{realm}/payment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboPaymentPayload {
    /// QBO Customer.Id.
    pub customer_ref: String,
    pub total_amount: f64,
    /// Transaction date in YYYY-MM-DD format.
    pub txn_date: Option<String>,
    /// ISO-4217 currency code.
    pub currency_ref: Option<String>,
    /// QBO PaymentMethod.Id.
    pub payment_method_ref: Option<String>,
    /// QBO Account.Id to deposit the payment into.
    pub deposit_to_account_ref: Option<String>,
    /// Invoice allocation lines. Empty = unapplied payment.
    /// All allocations are written in a single QBO call to prevent partial-apply.
    #[serde(default)]
    pub line_applications: Vec<PaymentLineApplication>,
}

impl QboPaymentPayload {
    pub(crate) fn to_qbo_json(&self) -> Value {
        let mut body = serde_json::json!({
            "CustomerRef": {"value": &self.customer_ref},
            "TotalAmt": self.total_amount,
        });
        if let Some(ref date) = self.txn_date {
            body["TxnDate"] = Value::String(date.clone());
        }
        if let Some(ref currency) = self.currency_ref {
            body["CurrencyRef"] = serde_json::json!({"value": currency});
        }
        if let Some(ref method) = self.payment_method_ref {
            body["PaymentMethodRef"] = serde_json::json!({"value": method});
        }
        if let Some(ref acct) = self.deposit_to_account_ref {
            body["DepositToAccountRef"] = serde_json::json!({"value": acct});
        }
        if !self.line_applications.is_empty() {
            let lines: Vec<Value> = self
                .line_applications
                .iter()
                .map(|la| {
                    serde_json::json!({
                        "Amount": la.amount,
                        "LinkedTxn": [{"TxnId": la.invoice_id, "TxnType": "Invoice"}]
                    })
                })
                .collect();
            body["Line"] = serde_json::json!(lines);
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

/// Fields excluded from the touched-field intent guard.
///
/// These are QBO system fields (SyncToken, MetaData) or local update hints
/// (sparse) that must never be treated as caller-touched business fields.
const SYSTEM_FIELDS_EXCLUDED: &[&str] =
    &["SyncToken", "MetaData", "sparse", "Id", "domain", "status"];

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

    /// Build a write URL. Appends `?minorversion=N&requestid=<caller-provided UUID>`.
    ///
    /// The caller must supply `request_id` from the ledger so that retries for
    /// the same ledger row reuse the same QBO idempotency key.
    pub(crate) fn write_url(&self, path: &str, request_id: Uuid) -> String {
        format!(
            "{}/{}?minorversion={}&requestid={}",
            self.company_url(),
            path,
            self.minor_version,
            request_id
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
        let retry_after = extract_retry_after(resp.headers());
        let body = resp.text().await?;

        if status == 200 {
            return parse_json(&body);
        }

        match classify_error(status, &body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                self.post_text(&url, statement, &new_token).await
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
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
    ///
    /// `request_id` must come from the ledger row so that retries for the same
    /// ledger operation reuse the same QBO idempotency key.
    pub async fn update_entity(
        &self,
        entity_type: &str,
        mut body: Value,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let entity_id = body["Id"]
            .as_str()
            .ok_or_else(|| QboError::Deserialize("update body missing Id".into()))?
            .to_string();

        // URL computed once so all retry attempts carry the same requestid.
        let url = self.write_url(&entity_type.to_lowercase(), request_id);

        for attempt in 0..=SYNC_TOKEN_MAX_RETRIES {
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
            let retry_after = extract_retry_after(resp.headers());
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
                    return self.post_json(&url, &body, &new_token).await;
                }
                QboApiAction::Backoff => return Err(QboError::RateLimited { retry_after }),
                QboApiAction::Fail => return Err(parse_api_error(&resp_body)),
            }
        }

        Err(QboError::SyncTokenExhausted(SYNC_TOKEN_MAX_RETRIES))
    }

    /// Update an entity with field-level intent guard on stale SyncToken retry.
    ///
    /// Like [`update_entity`] but adds a safety check on each stale retry:
    ///
    /// - `baseline`: the entity snapshot read **before** building the update body
    ///   (i.e. `get_entity` response at the entity key level, e.g. `response["Invoice"]`).
    ///   Pass `None` when no prior read is available.
    ///
    /// Guard behaviour on stale (5010):
    /// 1. Re-fetch the entity from QBO.
    /// 2. For each business field in `body` (excluding [`SYSTEM_FIELDS_EXCLUDED`]):
    ///    - If `baseline` is `Some(b)`: compare `b[field]` vs `fresh[field]`.
    ///      Any difference → [`QboError::ConflictDetected`] (someone else changed it).
    ///    - If `baseline` is `None` and the body contains any business field →
    ///      [`QboError::ConflictDetected`] (fail conservatively; can't verify safety).
    ///    - If no business fields present (only system fields) → safe to retry.
    /// 3. If no conflict is detected, update SyncToken and retry as normal.
    ///
    /// `request_id` must come from the ledger row for idempotency.
    pub async fn update_entity_with_guard(
        &self,
        entity_type: &str,
        mut body: Value,
        baseline: Option<&Value>,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let entity_id = body["Id"]
            .as_str()
            .ok_or_else(|| QboError::Deserialize("update body missing Id".into()))?
            .to_string();

        let url = self.write_url(&entity_type.to_lowercase(), request_id);
        let entity_key = capitalize(entity_type);

        for attempt in 0..=SYNC_TOKEN_MAX_RETRIES {
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
            let retry_after = extract_retry_after(resp.headers());
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
                        "SyncToken stale — running intent guard check"
                    );
                    let fresh = self.get_entity(entity_type, &entity_id).await?;
                    let fresh_entity = &fresh[&entity_key];

                    match baseline {
                        None if has_business_fields(&body) => {
                            return Err(QboError::ConflictDetected {
                                entity_id: entity_id.clone(),
                                fresh_entity: fresh_entity.clone(),
                            });
                        }
                        Some(bl) if touched_field_drifted(&body, bl, fresh_entity) => {
                            return Err(QboError::ConflictDetected {
                                entity_id: entity_id.clone(),
                                fresh_entity: fresh_entity.clone(),
                            });
                        }
                        _ => {}
                    }

                    if let Some(st) = fresh_entity["SyncToken"].as_str() {
                        body["SyncToken"] = Value::String(st.to_string());
                    }
                    continue;
                }
                QboApiAction::RetryWithFreshSyncToken => {
                    return Err(QboError::SyncTokenExhausted(SYNC_TOKEN_MAX_RETRIES));
                }
                QboApiAction::RefreshToken => {
                    let new_token = self.tokens.refresh_token().await?;
                    return self.post_json(&url, &body, &new_token).await;
                }
                QboApiAction::Backoff => return Err(QboError::RateLimited { retry_after }),
                QboApiAction::Fail => return Err(parse_api_error(&resp_body)),
            }
        }

        Err(QboError::SyncTokenExhausted(SYNC_TOKEN_MAX_RETRIES))
    }

    /// Create a new invoice in QBO.
    ///
    /// `request_id` must come from the ledger row so that transport-timeout
    /// retries reuse the same QBO idempotency key and never create duplicates.
    pub async fn create_invoice(
        &self,
        payload: &QboInvoicePayload,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        // URL computed once so token-expiry retries carry the same requestid.
        let url = self.write_url("invoice", request_id);
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
        let retry_after = extract_retry_after(resp.headers());
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Invoice"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let val = self.post_json(&url, &body, &new_token).await?;
                Ok(val["Invoice"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
            _ => Err(parse_api_error(&resp_body)),
        }
    }

    /// Create a new customer in QBO.
    ///
    /// `request_id` must come from the ledger row for idempotency.
    pub async fn create_customer(
        &self,
        payload: &QboCustomerPayload,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let url = self.write_url("customer", request_id);
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
        let retry_after = extract_retry_after(resp.headers());
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Customer"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let val = self.post_json(&url, &body, &new_token).await?;
                Ok(val["Customer"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
            _ => Err(parse_api_error(&resp_body)),
        }
    }

    /// Create a new payment in QBO.
    ///
    /// `request_id` must come from the ledger row for idempotency.
    pub async fn create_payment(
        &self,
        payload: &QboPaymentPayload,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let url = self.write_url("payment", request_id);
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
        let retry_after = extract_retry_after(resp.headers());
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Payment"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let val = self.post_json(&url, &body, &new_token).await?;
                Ok(val["Payment"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
            _ => Err(parse_api_error(&resp_body)),
        }
    }

    /// Void an invoice in QBO.
    ///
    /// QBO does not support hard-deleting invoices. The canonical operation is
    /// void: POST with `?operation=void` sets Balance=0 and locks the invoice.
    ///
    /// `request_id` must come from the ledger row for idempotency.
    pub async fn void_invoice(
        &self,
        qbo_id: &str,
        sync_token: &str,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let url = format!(
            "{}/invoice?operation=void&minorversion={}&requestid={}",
            self.company_url(),
            self.minor_version,
            request_id
        );
        let body = serde_json::json!({
            "Id": qbo_id,
            "SyncToken": sync_token,
        });
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
        let retry_after = extract_retry_after(resp.headers());
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Invoice"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let val = self.post_json(&url, &body, &new_token).await?;
                Ok(val["Invoice"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
            _ => Err(parse_api_error(&resp_body)),
        }
    }

    /// Delete a payment in QBO.
    ///
    /// QBO uses POST with `?operation=delete` (not Active=false like customers).
    ///
    /// `request_id` must come from the ledger row for idempotency.
    pub async fn delete_payment(
        &self,
        qbo_id: &str,
        sync_token: &str,
        request_id: Uuid,
    ) -> Result<Value, QboError> {
        let url = format!(
            "{}/payment?operation=delete&minorversion={}&requestid={}",
            self.company_url(),
            self.minor_version,
            request_id
        );
        let body = serde_json::json!({
            "Id": qbo_id,
            "SyncToken": sync_token,
        });
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
        let retry_after = extract_retry_after(resp.headers());
        let resp_body = resp.text().await?;

        if status == 200 {
            let val = parse_json(&resp_body)?;
            return Ok(val["Payment"].clone());
        }

        match classify_error(status, &resp_body) {
            QboApiAction::RefreshToken => {
                let new_token = self.tokens.refresh_token().await?;
                let val = self.post_json(&url, &body, &new_token).await?;
                Ok(val["Payment"].clone())
            }
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
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
        let retry_after = extract_retry_after(resp.headers());
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
            QboApiAction::Backoff => Err(QboError::RateLimited { retry_after }),
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

/// Extract the `Retry-After` value (seconds) from response headers.
fn extract_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
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

/// Returns true when `body` contains at least one key that is not a system field.
fn has_business_fields(body: &Value) -> bool {
    body.as_object()
        .map(|o| {
            o.keys()
                .any(|k| !SYSTEM_FIELDS_EXCLUDED.contains(&k.as_str()))
        })
        .unwrap_or(false)
}

/// Returns true when any touched business field in `body` has a different value
/// in `fresh_entity` than in `baseline_entity`.
///
/// Touched fields = keys present in `body` that are not in [`SYSTEM_FIELDS_EXCLUDED`].
/// A difference between baseline and fresh means a concurrent writer changed that
/// field after our last read.
fn touched_field_drifted(body: &Value, baseline_entity: &Value, fresh_entity: &Value) -> bool {
    let body_obj = match body.as_object() {
        Some(o) => o,
        None => return false,
    };
    for field in body_obj.keys() {
        if SYSTEM_FIELDS_EXCLUDED.contains(&field.as_str()) {
            continue;
        }
        if baseline_entity[field] != fresh_entity[field] {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

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
    fn write_url_encodes_caller_requestid() {
        let c = test_client("https://sandbox-quickbooks.api.intuit.com/v3");
        let id = Uuid::new_v4();
        let url = c.write_url("invoice", id);
        assert!(url.contains("minorversion=75"), "missing minorversion");
        let rid = url.split("requestid=").nth(1).expect("requestid param");
        assert_eq!(rid, id.to_string().as_str(), "requestid must equal caller-provided UUID");
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

    #[test]
    fn extract_retry_after_parses_seconds() {
        let mut map = reqwest::header::HeaderMap::new();
        map.insert(
            "retry-after",
            reqwest::header::HeaderValue::from_static("42"),
        );
        assert_eq!(extract_retry_after(&map), Some(Duration::from_secs(42)));
    }

    #[test]
    fn extract_retry_after_missing_returns_none() {
        assert_eq!(extract_retry_after(&reqwest::header::HeaderMap::new()), None);
    }

    #[test]
    fn qbo_customer_payload_serializes_correctly() {
        let p = QboCustomerPayload {
            display_name: "Acme Corp".into(),
            email: Some("billing@acme.com".into()),
            company_name: Some("Acme Corporation".into()),
            currency_ref: Some("USD".into()),
        };
        let j = p.to_qbo_json();
        assert_eq!(j["DisplayName"].as_str(), Some("Acme Corp"));
        assert_eq!(j["PrimaryEmailAddr"]["Address"].as_str(), Some("billing@acme.com"));
        assert_eq!(j["CompanyName"].as_str(), Some("Acme Corporation"));
        assert_eq!(j["CurrencyRef"]["value"].as_str(), Some("USD"));
    }

    #[test]
    fn qbo_payment_payload_serializes_correctly() {
        let p = QboPaymentPayload {
            customer_ref: "7".into(),
            total_amount: 250.00,
            txn_date: Some("2026-04-20".into()),
            currency_ref: Some("USD".into()),
            payment_method_ref: Some("2".into()),
            deposit_to_account_ref: Some("35".into()),
        };
        let j = p.to_qbo_json();
        assert_eq!(j["CustomerRef"]["value"].as_str(), Some("7"));
        assert_eq!(j["TotalAmt"].as_f64(), Some(250.00));
        assert_eq!(j["TxnDate"].as_str(), Some("2026-04-20"));
        assert_eq!(j["CurrencyRef"]["value"].as_str(), Some("USD"));
        assert_eq!(j["PaymentMethodRef"]["value"].as_str(), Some("2"));
        assert_eq!(j["DepositToAccountRef"]["value"].as_str(), Some("35"));
    }

    // These retry tests use a local axum server because the sandbox cannot
    // force the same 5xx sequence on demand.

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

        let result = client.update_entity("Invoice", body, Uuid::new_v4()).await;
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

        let result = client.update_entity("Invoice", body, Uuid::new_v4()).await;
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

    // -- requestid determinism test --

    #[derive(Clone)]
    struct RecordingState {
        recorded_ids: Arc<Mutex<Vec<String>>>,
        post_count: Arc<AtomicU32>,
        max_failures: u32,
    }

    async fn recording_get(
        axum::extract::State(_): axum::extract::State<RecordingState>,
    ) -> (axum::http::StatusCode, String) {
        (
            axum::http::StatusCode::OK,
            r#"{"Invoice":{"Id":"1","SyncToken":"99"}}"#.into(),
        )
    }

    async fn recording_post(
        axum::extract::State(s): axum::extract::State<RecordingState>,
        uri: axum::http::Uri,
    ) -> (axum::http::StatusCode, String) {
        let n = s.post_count.fetch_add(1, Ordering::SeqCst);
        if let Some(query) = uri.query() {
            for part in query.split('&') {
                if let Some(id) = part.strip_prefix("requestid=") {
                    s.recorded_ids.lock().expect("test mutex").push(id.to_string());
                    break;
                }
            }
        }
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

    #[tokio::test]
    async fn update_entity_same_requestid_on_retry() {
        let state = RecordingState {
            recorded_ids: Arc::new(Mutex::new(Vec::new())),
            post_count: Arc::new(AtomicU32::new(0)),
            max_failures: 1,
        };
        let app = axum::Router::new()
            .route(
                "/v3/company/{realm}/invoice/{id}",
                axum::routing::get(recording_get),
            )
            .route(
                "/v3/company/{realm}/invoice",
                axum::routing::post(recording_post),
            )
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
        let addr = listener.local_addr().expect("test local addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("test server") });

        let client = test_client(&format!("http://{}/v3", addr));
        let rid = Uuid::new_v4();
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true});

        let result = client.update_entity("Invoice", body, rid).await;
        assert!(result.is_ok(), "expected success: {:?}", result);

        let ids = state.recorded_ids.lock().expect("test mutex");
        assert_eq!(ids.len(), 2, "expected 2 POST attempts");
        assert_eq!(
            ids[0], ids[1],
            "requestid must be identical across retries, got: {:?}",
            &*ids
        );
        assert_eq!(
            ids[0],
            rid.to_string(),
            "requestid must match caller-provided UUID"
        );
    }

    // -- create_invoice tests --

    async fn start_create_server() -> (String, Arc<AtomicU32>) {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let app = axum::Router::new().route(
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
        tokio::spawn(async move { axum::serve(listener, app).await.expect("server crashed") });
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
            currency_ref: None,
        };

        let result = client
            .create_invoice(&payload, Uuid::new_v4())
            .await
            .expect("create_invoice failed");
        assert_eq!(
            result["Id"].as_str(),
            Some("42"),
            "returned invoice must have Id=42"
        );
        assert_eq!(call_count.load(Ordering::SeqCst), 1, "one POST to QBO");
    }

    #[tokio::test]
    async fn create_customer_returns_customer_id() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let app = axum::Router::new().route(
            "/v3/company/{realm}/customer",
            axum::routing::post(move || {
                let c = count.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::OK,
                        r#"{"Customer":{"Id":"99","SyncToken":"0","DisplayName":"Acme Corp"}}"#,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
        let addr = listener.local_addr().expect("test local addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("test server") });

        let client = test_client(&format!("http://{}/v3", addr));
        let payload = QboCustomerPayload {
            display_name: "Acme Corp".into(),
            email: Some("billing@acme.com".into()),
            company_name: None,
            currency_ref: None,
        };
        let result = client
            .create_customer(&payload, Uuid::new_v4())
            .await
            .expect("create_customer failed");
        assert_eq!(result["Id"].as_str(), Some("99"));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn create_payment_returns_payment_id() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let app = axum::Router::new().route(
            "/v3/company/{realm}/payment",
            axum::routing::post(move || {
                let c = count.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    (
                        axum::http::StatusCode::OK,
                        r#"{"Payment":{"Id":"55","SyncToken":"0","TotalAmt":250.0}}"#,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
        let addr = listener.local_addr().expect("test local addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("test server") });

        let client = test_client(&format!("http://{}/v3", addr));
        let payload = QboPaymentPayload {
            customer_ref: "7".into(),
            total_amount: 250.00,
            txn_date: Some("2026-04-20".into()),
            currency_ref: None,
            payment_method_ref: None,
            deposit_to_account_ref: None,
        };
        let result = client
            .create_payment(&payload, Uuid::new_v4())
            .await
            .expect("create_payment failed");
        assert_eq!(result["Id"].as_str(), Some("55"));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
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
            currency_ref: None,
        };
        let json = payload.to_qbo_json();
        assert_eq!(json["CustomerRef"]["value"].as_str(), Some("5"));
        assert_eq!(json["DueDate"].as_str(), Some("2026-06-15"));
        assert_eq!(json["DocNumber"].as_str(), Some("DOC-999"));
        assert!(json.get("CurrencyRef").is_none(), "absent currency_ref must not emit field");
        let lines = json["Line"].as_array().expect("Line must be array");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["Amount"].as_f64(), Some(500.00));
        assert_eq!(
            lines[0]["SalesItemLineDetail"]["ItemRef"]["value"].as_str(),
            Some("1")
        );
    }

    #[test]
    fn qbo_invoice_payload_currency_ref_serializes() {
        let payload = QboInvoicePayload {
            customer_ref: "3".to_string(),
            line_items: vec![],
            due_date: None,
            doc_number: None,
            currency_ref: Some("EUR".to_string()),
        };
        let json = payload.to_qbo_json();
        assert_eq!(json["CurrencyRef"]["value"].as_str(), Some("EUR"));
    }

    #[tokio::test]
    async fn void_invoice_url_contains_operation_void() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();
        let app = axum::Router::new().route(
            "/v3/company/{realm}/invoice",
            axum::routing::post(move |uri: axum::http::Uri| {
                let c = count.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    let q = uri.query().unwrap_or("");
                    assert!(q.contains("operation=void"), "missing operation=void in {q}");
                    assert!(q.contains("minorversion=75"), "missing minorversion in {q}");
                    (
                        axum::http::StatusCode::OK,
                        r#"{"Invoice":{"Id":"10","SyncToken":"1","Balance":0.0}}"#,
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("server") });

        let client = test_client(&format!("http://{}/v3", addr));
        let result = client.void_invoice("10", "0", Uuid::new_v4()).await.expect("void_invoice");
        assert_eq!(result["Id"].as_str(), Some("10"));
        assert_eq!(result["Balance"].as_f64(), Some(0.0));
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    // ── has_business_fields unit tests ────────────────────────────────────────

    #[test]
    fn has_business_fields_only_system_fields_is_false() {
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true});
        assert!(!has_business_fields(&body));
    }

    #[test]
    fn has_business_fields_with_ship_date_is_true() {
        let body =
            serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true, "ShipDate": "2026-04-20"});
        assert!(has_business_fields(&body));
    }

    #[test]
    fn touched_field_drifted_returns_false_when_no_drift() {
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "ShipDate": "2026-04-20"});
        let baseline = serde_json::json!({"ShipDate": "2026-04-01", "Amount": 100.0});
        let fresh = serde_json::json!({"SyncToken": "7", "ShipDate": "2026-04-01", "Amount": 100.0});
        // ShipDate unchanged between baseline and fresh → no drift
        assert!(!touched_field_drifted(&body, &baseline, &fresh));
    }

    #[test]
    fn touched_field_drifted_returns_true_when_touched_field_changed() {
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "ShipDate": "2026-04-20"});
        let baseline = serde_json::json!({"ShipDate": "2026-04-01", "Amount": 100.0});
        let fresh = serde_json::json!({"SyncToken": "7", "ShipDate": "2026-04-25", "Amount": 100.0});
        // ShipDate changed between baseline and fresh while we're touching it → drift
        assert!(touched_field_drifted(&body, &baseline, &fresh));
    }

    #[test]
    fn touched_field_drifted_ignores_untouched_fields() {
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "ShipDate": "2026-04-20"});
        let baseline = serde_json::json!({"ShipDate": "2026-04-01", "Amount": 100.0});
        // Amount changed (not in body) → should not trigger drift
        let fresh = serde_json::json!({"SyncToken": "7", "ShipDate": "2026-04-01", "Amount": 200.0});
        assert!(!touched_field_drifted(&body, &baseline, &fresh));
    }

    // ── update_entity_with_guard integration tests (local axum server) ────────

    #[derive(Clone)]
    struct GuardTestState {
        post_count: Arc<AtomicU32>,
        get_count: Arc<AtomicU32>,
        /// If true the GET response returns a changed ShipDate vs baseline.
        ship_date_drifted: bool,
        max_failures: u32,
    }

    async fn guard_post(
        axum::extract::State(s): axum::extract::State<GuardTestState>,
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
                r#"{"Invoice":{"Id":"1","SyncToken":"10","ShipDate":"2026-04-20"}}"#.into(),
            )
        }
    }

    async fn guard_get(
        axum::extract::State(s): axum::extract::State<GuardTestState>,
    ) -> (axum::http::StatusCode, String) {
        s.get_count.fetch_add(1, Ordering::SeqCst);
        let ship_date = if s.ship_date_drifted {
            "2026-04-25" // someone else changed it
        } else {
            "2026-04-01" // unchanged from baseline
        };
        let body = format!(
            r#"{{"Invoice":{{"Id":"1","SyncToken":"9","ShipDate":"{}"}}}}"#,
            ship_date
        );
        (axum::http::StatusCode::OK, body)
    }

    async fn start_guard_server(
        ship_date_drifted: bool,
        max_failures: u32,
    ) -> (String, GuardTestState) {
        let state = GuardTestState {
            post_count: Arc::new(AtomicU32::new(0)),
            get_count: Arc::new(AtomicU32::new(0)),
            ship_date_drifted,
            max_failures,
        };
        let app = axum::Router::new()
            .route("/v3/company/{realm}/invoice/{id}", axum::routing::get(guard_get))
            .route("/v3/company/{realm}/invoice", axum::routing::post(guard_post))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind test server");
        let addr = listener.local_addr().expect("test local addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("test server") });
        (format!("http://{}/v3", addr), state)
    }

    #[tokio::test]
    async fn guard_no_drift_retries_successfully() {
        // Baseline ShipDate == fresh ShipDate → no conflict → retry succeeds
        let (base_url, state) = start_guard_server(false, 1).await;
        let client = test_client(&base_url);
        let body =
            serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true, "ShipDate": "2026-04-20"});
        let baseline = serde_json::json!({"ShipDate": "2026-04-01", "SyncToken": "5"});

        let result = client
            .update_entity_with_guard("Invoice", body, Some(&baseline), Uuid::new_v4())
            .await;
        assert!(result.is_ok(), "expected success: {:?}", result);
        assert_eq!(state.post_count.load(Ordering::SeqCst), 2, "2 POST attempts");
        assert_eq!(state.get_count.load(Ordering::SeqCst), 1, "1 GET re-fetch");
    }

    #[tokio::test]
    async fn guard_touched_field_drift_returns_conflict_detected() {
        // Fresh entity has different ShipDate from baseline → ConflictDetected
        let (base_url, _state) = start_guard_server(true, 1).await;
        let client = test_client(&base_url);
        let body =
            serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true, "ShipDate": "2026-04-20"});
        let baseline = serde_json::json!({"ShipDate": "2026-04-01", "SyncToken": "5"});

        let result = client
            .update_entity_with_guard("Invoice", body, Some(&baseline), Uuid::new_v4())
            .await;
        assert!(
            matches!(result, Err(QboError::ConflictDetected { .. })),
            "expected ConflictDetected: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn guard_no_baseline_business_fields_fails_conservatively() {
        // No baseline + body has business fields → fail conservatively
        let (base_url, _state) = start_guard_server(false, 1).await;
        let client = test_client(&base_url);
        let body =
            serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true, "ShipDate": "2026-04-20"});

        let result = client
            .update_entity_with_guard("Invoice", body, None, Uuid::new_v4())
            .await;
        assert!(
            matches!(result, Err(QboError::ConflictDetected { .. })),
            "expected ConflictDetected: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn guard_no_baseline_only_system_fields_retries_safely() {
        // No baseline + body has ONLY system fields → safe retry
        let (base_url, state) = start_guard_server(false, 1).await;
        let client = test_client(&base_url);
        let body = serde_json::json!({"Id": "1", "SyncToken": "5", "sparse": true});

        let result = client
            .update_entity_with_guard("Invoice", body, None, Uuid::new_v4())
            .await;
        assert!(result.is_ok(), "expected success: {:?}", result);
        assert_eq!(state.post_count.load(Ordering::SeqCst), 2);
    }
}
