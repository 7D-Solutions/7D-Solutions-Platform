//! GL HTTP client adapter — wraps platform-client-gl with consolidation-specific methods.
//!
//! Uses `PlatformClient` for tenant header injection, correlation IDs, and
//! retry on 429/503.  Response types are defined here until the generated
//! client skeleton supports typed responses for these endpoints.

use platform_sdk::{build_query_url, parse_response, ClientError, PlatformClient};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Trial balance row as returned by GL's GET /api/gl/trial-balance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlTrialBalanceRow {
    pub account_code: String,
    pub account_name: String,
    pub account_type: String,
    pub normal_balance: String,
    pub currency: String,
    pub debit_total_minor: i64,
    pub credit_total_minor: i64,
    pub net_balance_minor: i64,
}

/// Full GL trial balance response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlTrialBalanceResponse {
    pub tenant_id: String,
    pub period_id: Uuid,
    pub currency: String,
    pub rows: Vec<GlTrialBalanceRow>,
    pub totals: GlStatementTotals,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlStatementTotals {
    pub total_debits: i64,
    pub total_credits: i64,
    pub is_balanced: bool,
}

/// Period close status as returned by GL's GET /api/gl/periods/{id}/close-status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlCloseStatusResponse {
    pub period_id: Uuid,
    pub tenant_id: String,
    pub period_start: String,
    pub period_end: String,
    pub close_status: GlCloseStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GlCloseStatus {
    Open,
    CloseRequested {
        requested_at: String,
    },
    Closed {
        closed_at: String,
        closed_by: String,
        close_reason: Option<String>,
        close_hash: String,
        requested_at: Option<String>,
    },
}

/// FX rate response from GL's GET /api/gl/fx-rates/latest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlFxRateResponse {
    pub rate: f64,
    pub base_currency: String,
    pub quote_currency: String,
}

/// Response from GL journal entry creation endpoint.
#[derive(Debug, Clone, Deserialize)]
struct PostJournalResponse {
    pub journal_entry_id: Uuid,
}

// ── Query parameter structs ─────────────────────────────────────

#[derive(Serialize)]
struct TrialBalanceQuery<'a> {
    period_id: Uuid,
    currency: &'a str,
}

#[derive(Serialize)]
struct FxRateQuery<'a> {
    base_currency: &'a str,
    quote_currency: &'a str,
    as_of: &'a str,
}

/// HTTP client adapter for the GL service.
///
/// Uses `PlatformClient` from `platform-sdk` for tenant header injection,
/// correlation IDs, and automatic retry on 429/503 for GET requests.
#[derive(Clone)]
pub struct GlClient {
    client: PlatformClient,
}

impl platform_sdk::PlatformService for GlClient {
    const SERVICE_NAME: &'static str = "gl";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self { client }
    }
}

impl GlClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: PlatformClient::new(base_url.trim_end_matches('/').to_string()),
        }
    }

    /// Parse a tenant ID string into a UUID for service claims.
    fn parse_tenant(tenant_id: &str) -> Result<Uuid, ClientError> {
        Uuid::parse_str(tenant_id).map_err(|e| ClientError::Unexpected {
            status: 0,
            body: format!("invalid tenant_id: {e}"),
        })
    }

    /// Fetch trial balance for an entity (tenant) + period + currency.
    pub async fn get_trial_balance(
        &self,
        tenant_id: &str,
        period_id: Uuid,
        currency: &str,
    ) -> Result<GlTrialBalanceResponse, ClientError> {
        let claims = PlatformClient::service_claims(Self::parse_tenant(tenant_id)?);
        let path = build_query_url(
            "/api/gl/trial-balance",
            &TrialBalanceQuery {
                period_id,
                currency,
            },
        )?;
        let resp = self
            .client
            .get(&path, &claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// Fetch period close status to get the close_hash for verification.
    pub async fn get_close_status(
        &self,
        tenant_id: &str,
        period_id: Uuid,
    ) -> Result<GlCloseStatusResponse, ClientError> {
        let claims = PlatformClient::service_claims(Self::parse_tenant(tenant_id)?);
        let path = format!("/api/gl/periods/{}/close-status", period_id);
        let resp = self
            .client
            .get(&path, &claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// Fetch close hash for a period. Returns None if the period is not closed.
    pub async fn get_close_hash(
        &self,
        tenant_id: &str,
        period_id: Uuid,
    ) -> Result<Option<String>, ClientError> {
        let status = self.get_close_status(tenant_id, period_id).await?;
        match status.close_status {
            GlCloseStatus::Closed { close_hash, .. } => Ok(Some(close_hash)),
            _ => Ok(None),
        }
    }

    /// Fetch the latest FX rate for a currency pair as-of a given date.
    ///
    /// Returns None if no rate is found (404).
    pub async fn get_fx_rate(
        &self,
        tenant_id: &str,
        base_currency: &str,
        quote_currency: &str,
        as_of: &str,
    ) -> Result<Option<GlFxRateResponse>, ClientError> {
        let claims = PlatformClient::service_claims(Self::parse_tenant(tenant_id)?);
        let path = build_query_url(
            "/api/gl/fx-rates/latest",
            &FxRateQuery {
                base_currency,
                quote_currency,
                as_of,
            },
        )?;
        let resp = self
            .client
            .get(&path, &claims)
            .await
            .map_err(ClientError::Network)?;

        if resp.status().as_u16() == 404 {
            return Ok(None);
        }

        parse_response(resp).await.map(Some)
    }

    /// Post an elimination journal entry to GL.
    ///
    /// Uses source_module = "consolidation-elimination" for audit trail.
    /// The source_doc_id acts as an idempotency reference within GL.
    pub async fn post_elimination_journal(
        &self,
        tenant_id: &str,
        posting_date: &str,
        currency: &str,
        debit_account: &str,
        credit_account: &str,
        amount_minor: i64,
        description: &str,
        source_doc_id: &str,
    ) -> Result<Uuid, ClientError> {
        let claims = PlatformClient::service_claims(Self::parse_tenant(tenant_id)?);
        let amount_major = amount_minor as f64 / 100.0;

        let body = serde_json::json!({
            "tenant_id": tenant_id,
            "source_module": "consolidation-elimination",
            "posting_date": posting_date,
            "currency": currency,
            "source_doc_type": "GL_ACCRUAL",
            "source_doc_id": source_doc_id,
            "description": description,
            "lines": [
                {
                    "account_ref": debit_account,
                    "debit": amount_major,
                    "credit": 0,
                    "memo": format!("Elimination DR: {}", description)
                },
                {
                    "account_ref": credit_account,
                    "debit": 0,
                    "credit": amount_major,
                    "memo": format!("Elimination CR: {}", description)
                }
            ]
        });

        let resp = self
            .client
            .post("/api/gl/journal-entries", &body, &claims)
            .await
            .map_err(ClientError::Network)?;
        let result: PostJournalResponse = parse_response(resp).await?;
        Ok(result.journal_entry_id)
    }
}
