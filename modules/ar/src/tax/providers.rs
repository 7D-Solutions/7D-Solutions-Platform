//! Tax provider implementations — ZeroTaxProvider (testing), LocalTaxProvider (deterministic),
//! and AvalaraProvider (live AvaTax REST API).

use base64::{engine::general_purpose::STANDARD as BASE64_STD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tax_core::models::*;
use tax_core::{TaxProvider, TaxProviderError};

// ============================================================================
// Stub implementation for testing
// ============================================================================

/// No-op tax provider that returns zero tax for all requests.
/// Used in tests and local development where no live provider is configured.
pub struct ZeroTaxProvider;

impl TaxProvider for ZeroTaxProvider {
    async fn quote_tax(&self, req: TaxQuoteRequest) -> Result<TaxQuoteResponse, TaxProviderError> {
        let zero_lines: Vec<TaxByLine> = req
            .line_items
            .iter()
            .map(|l| TaxByLine {
                line_id: l.line_id.clone(),
                tax_minor: 0,
                rate: 0.0,
                jurisdiction: "zero-tax".to_string(),
                tax_type: "none".to_string(),
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor: 0,
            tax_by_line: zero_lines,
            provider_quote_ref: format!("zero-quote-{}", Uuid::new_v4()),
            expires_at: None,
            quoted_at: Utc::now(),
            warnings: vec![],
        })
    }

    async fn commit_tax(
        &self,
        _req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        Ok(TaxCommitResponse {
            provider_commit_ref: format!("zero-commit-{}", Uuid::new_v4()),
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(&self, _req: TaxVoidRequest) -> Result<TaxVoidResponse, TaxProviderError> {
        Ok(TaxVoidResponse {
            voided: true,
            voided_at: Utc::now(),
        })
    }
}

// ============================================================================
// Local deterministic tax provider (bd-29j)
// ============================================================================

/// Deterministic tax provider for E2E testing and local development.
///
/// Calculates tax based on the destination state using a fixed rate table.
/// Rates are hardcoded to ensure deterministic, reproducible results across
/// test runs without requiring an external tax service.
///
/// Rate table (US states, ship_to.state):
/// - CA: 8.5% (California)
/// - NY: 8.0% (New York)
/// - TX: 6.25% (Texas)
/// - WA: 6.5% (Washington)
/// - FL: 6.0% (Florida)
/// - All others: 5.0% (default)
///
/// Non-US countries: 0% (tax-exempt in local provider)
pub struct LocalTaxProvider;

impl LocalTaxProvider {
    /// Resolve the tax rate for a given state code.
    /// Returns (rate, jurisdiction_name).
    fn resolve_rate(state: &str, country: &str) -> (f64, String) {
        if country != "US" {
            return (0.0, format!("{} (exempt)", country));
        }
        match state.to_uppercase().as_str() {
            "CA" => (0.085, "California State Tax".to_string()),
            "NY" => (0.08, "New York State Tax".to_string()),
            "TX" => (0.0625, "Texas State Tax".to_string()),
            "WA" => (0.065, "Washington State Tax".to_string()),
            "FL" => (0.06, "Florida State Tax".to_string()),
            other => (0.05, format!("{} Default Tax", other)),
        }
    }
}

impl TaxProvider for LocalTaxProvider {
    async fn quote_tax(&self, req: TaxQuoteRequest) -> Result<TaxQuoteResponse, TaxProviderError> {
        if req.line_items.is_empty() {
            return Err(TaxProviderError::InvalidRequest(
                "No line items provided".to_string(),
            ));
        }

        let (rate, jurisdiction) = Self::resolve_rate(&req.ship_to.state, &req.ship_to.country);

        let mut total_tax: i64 = 0;
        let tax_by_line: Vec<TaxByLine> = req
            .line_items
            .iter()
            .map(|l| {
                // Banker's rounding: (amount * rate + 0.5).floor()
                let tax = ((l.amount_minor as f64) * rate).round() as i64;
                total_tax += tax;
                TaxByLine {
                    line_id: l.line_id.clone(),
                    tax_minor: tax,
                    rate,
                    jurisdiction: jurisdiction.clone(),
                    tax_type: "sales_tax".to_string(),
                }
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor: total_tax,
            tax_by_line,
            provider_quote_ref: format!("local-quote-{}", Uuid::new_v4()),
            expires_at: None,
            quoted_at: Utc::now(),
            warnings: vec![],
        })
    }

    async fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        if !req.provider_quote_ref.starts_with("local-quote-") {
            return Err(TaxProviderError::CommitRejected(
                "Unknown quote reference".to_string(),
            ));
        }
        Ok(TaxCommitResponse {
            provider_commit_ref: format!("local-commit-{}", Uuid::new_v4()),
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(&self, req: TaxVoidRequest) -> Result<TaxVoidResponse, TaxProviderError> {
        if !req.provider_commit_ref.starts_with("local-commit-") {
            return Err(TaxProviderError::VoidRejected(
                "Unknown commit reference".to_string(),
            ));
        }
        Ok(TaxVoidResponse {
            voided: true,
            voided_at: Utc::now(),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_address() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".to_string(),
            line2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94102".to_string(),
            country: "US".to_string(),
        }
    }

    fn sample_line() -> TaxLineItem {
        TaxLineItem {
            line_id: "line-1".to_string(),
            description: "SaaS subscription".to_string(),
            amount_minor: 10000,
            currency: "usd".to_string(),
            tax_code: Some("SW050000".to_string()),
            quantity: 1.0,
        }
    }

    fn sample_quote_req() -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            customer_id: "cust-1".to_string(),
            ship_to: sample_address(),
            ship_from: sample_address(),
            line_items: vec![sample_line()],
            currency: "usd".to_string(),
            invoice_date: Utc::now(),
            correlation_id: "corr-1".to_string(),
        }
    }

    #[tokio::test]
    async fn zero_tax_provider_returns_zero_tax() -> Result<(), Box<dyn std::error::Error>> {
        let provider = ZeroTaxProvider;
        let response = provider.quote_tax(sample_quote_req()).await?;
        assert_eq!(response.total_tax_minor, 0);
        assert_eq!(response.tax_by_line.len(), 1);
        assert_eq!(response.tax_by_line[0].tax_minor, 0);
        assert_eq!(response.tax_by_line[0].rate, 0.0);
        assert!(!response.provider_quote_ref.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn zero_tax_provider_commit_succeeds() -> Result<(), Box<dyn std::error::Error>> {
        let provider = ZeroTaxProvider;
        let req = TaxCommitRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            provider_quote_ref: "quote-abc".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let resp = provider.commit_tax(req).await?;
        assert!(!resp.provider_commit_ref.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn zero_tax_provider_void_succeeds() -> Result<(), Box<dyn std::error::Error>> {
        let provider = ZeroTaxProvider;
        let req = TaxVoidRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            provider_commit_ref: "commit-abc".to_string(),
            void_reason: "invoice_cancelled".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let resp = provider.void_tax(req).await?;
        assert!(resp.voided);
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_california_rate() -> Result<(), Box<dyn std::error::Error>> {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "CA".to_string();
        let resp = provider.quote_tax(req).await?;
        // 10000 * 0.085 = 850
        assert_eq!(resp.total_tax_minor, 850);
        assert_eq!(resp.tax_by_line[0].rate, 0.085);
        assert_eq!(resp.tax_by_line[0].jurisdiction, "California State Tax");
        assert!(resp.provider_quote_ref.starts_with("local-quote-"));
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_new_york_rate() -> Result<(), Box<dyn std::error::Error>> {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "NY".to_string();
        let resp = provider.quote_tax(req).await?;
        // 10000 * 0.08 = 800
        assert_eq!(resp.total_tax_minor, 800);
        assert_eq!(resp.tax_by_line[0].rate, 0.08);
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_default_rate() -> Result<(), Box<dyn std::error::Error>> {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "MT".to_string(); // Montana — not in rate table
        let resp = provider.quote_tax(req).await?;
        // 10000 * 0.05 = 500
        assert_eq!(resp.total_tax_minor, 500);
        assert_eq!(resp.tax_by_line[0].rate, 0.05);
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_non_us_exempt() -> Result<(), Box<dyn std::error::Error>> {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.country = "GB".to_string();
        let resp = provider.quote_tax(req).await?;
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line[0].rate, 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_multi_line() -> Result<(), Box<dyn std::error::Error>> {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "CA".to_string();
        req.line_items.push(TaxLineItem {
            line_id: "line-2".to_string(),
            description: "Storage addon".to_string(),
            amount_minor: 5000,
            currency: "usd".to_string(),
            tax_code: None,
            quantity: 1.0,
        });
        let resp = provider.quote_tax(req).await?;
        // 10000 * 0.085 = 850, 5000 * 0.085 = 425 → total 1275
        assert_eq!(resp.total_tax_minor, 1275);
        assert_eq!(resp.tax_by_line.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn local_provider_empty_lines_rejected() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.line_items.clear();
        let err = provider.quote_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn local_provider_commit_rejects_unknown_ref() {
        let provider = LocalTaxProvider;
        let req = TaxCommitRequest {
            tenant_id: "t".to_string(),
            invoice_id: "i".to_string(),
            provider_quote_ref: "avalara-quote-123".to_string(),
            correlation_id: "c".to_string(),
        };
        let err = provider.commit_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::CommitRejected(_)));
    }

    #[tokio::test]
    async fn local_provider_void_rejects_unknown_ref() {
        let provider = LocalTaxProvider;
        let req = TaxVoidRequest {
            tenant_id: "t".to_string(),
            invoice_id: "i".to_string(),
            provider_commit_ref: "avalara-commit-123".to_string(),
            void_reason: "test".to_string(),
            correlation_id: "c".to_string(),
        };
        let err = provider.void_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::VoidRejected(_)));
    }
}

// ============================================================================
// Avalara AvaTax provider
// ============================================================================

/// Configuration for the Avalara AvaTax provider.
///
/// Load via `AvalaraConfig::from_env()` or construct directly for testing.
#[derive(Debug, Clone)]
pub struct AvalaraConfig {
    /// Avalara account ID (numeric string)
    pub account_id: String,
    /// Avalara license key
    pub license_key: String,
    /// Avalara company code (e.g. "DEFAULT")
    pub company_code: String,
    /// AvaTax REST base URL. Sandbox: https://sandbox-rest.avatax.com
    pub base_url: String,
    /// HTTP request timeout in seconds
    pub timeout_secs: u64,
}

impl AvalaraConfig {
    /// Load from environment variables.
    ///
    /// Required: `AVALARA_ACCOUNT_ID`, `AVALARA_LICENSE_KEY`, `AVALARA_COMPANY_CODE`
    /// Optional: `AVALARA_BASE_URL` (default: https://sandbox-rest.avatax.com)
    pub fn from_env() -> Result<Self, std::env::VarError> {
        Ok(Self {
            account_id: std::env::var("AVALARA_ACCOUNT_ID")?,
            license_key: std::env::var("AVALARA_LICENSE_KEY")?,
            company_code: std::env::var("AVALARA_COMPANY_CODE")?,
            base_url: std::env::var("AVALARA_BASE_URL")
                .unwrap_or_else(|_| "https://sandbox-rest.avatax.com".to_string()),
            timeout_secs: 30,
        })
    }
}

/// Avalara AvaTax implementation of [`TaxProvider`].
///
/// - `quote_tax`  → `POST /api/v2/transactions/create` (type=SalesOrder, not committed)
/// - `commit_tax` → `POST /api/v2/companies/{co}/transactions/{code}/commit`
/// - `void_tax`   → `POST /api/v2/companies/{co}/transactions/{code}/void`
///
/// HTTP 4xx responses map to [`TaxProviderError::Provider`] (permanent — do not retry).
/// Timeouts, connect failures, and 5xx map to [`TaxProviderError::Unavailable`] (retry-safe).
pub struct AvalaraProvider {
    client: reqwest::Client,
    base_url: String,
    account_id: String,
    license_key: String,
    company_code: String,
}

impl AvalaraProvider {
    pub fn new(cfg: AvalaraConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs))
            .build()
            .expect("reqwest client build failed");
        Self {
            client,
            base_url: cfg.base_url,
            account_id: cfg.account_id,
            license_key: cfg.license_key,
            company_code: cfg.company_code,
        }
    }

    fn auth_header(&self) -> String {
        let creds = format!("{}:{}", self.account_id, self.license_key);
        format!("Basic {}", BASE64_STD.encode(creds.as_bytes()))
    }
}

// --- Internal Avalara request/response types (not exposed outside this module) ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraCreateRequest {
    #[serde(rename = "type")]
    transaction_type: String,
    code: String,
    company_code: String,
    date: String,
    customer_code: String,
    addresses: AvalaraAddresses,
    lines: Vec<AvalaraLine>,
    currency_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<bool>,
    reference_code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraAddresses {
    ship_from: AvalaraAddress,
    ship_to: AvalaraAddress,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraAddress {
    line1: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line2: Option<String>,
    city: String,
    region: String,
    postal_code: String,
    country: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraLine {
    number: String,
    amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tax_code: Option<String>,
    description: String,
    quantity: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraTransactionResponse {
    code: String,
    total_tax: f64,
    #[serde(default)]
    lines: Vec<AvalaraTransactionLine>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraTransactionLine {
    tax: f64,
    #[serde(default)]
    details: Vec<AvalaraLineDetail>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvalaraLineDetail {
    tax_name: String,
    tax_type: String,
    rate: f64,
}

#[derive(Serialize)]
struct AvalaraCommitRequest {
    commit: bool,
}

#[derive(Serialize)]
struct AvalaraVoidRequest {
    code: String,
}

fn map_address(addr: &TaxAddress) -> AvalaraAddress {
    AvalaraAddress {
        line1: addr.line1.clone(),
        line2: addr.line2.clone(),
        city: addr.city.clone(),
        region: addr.state.clone(),
        postal_code: addr.postal_code.clone(),
        country: addr.country.clone(),
    }
}

fn map_reqwest_err(e: reqwest::Error) -> TaxProviderError {
    if e.is_timeout() || e.is_connect() {
        TaxProviderError::Unavailable(format!("Avalara network error: {}", e))
    } else {
        TaxProviderError::Provider(format!("Avalara request error: {}", e))
    }
}

async fn expect_avalara_success(
    resp: reqwest::Response,
) -> Result<reqwest::Response, TaxProviderError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    if status.is_client_error() {
        Err(TaxProviderError::Provider(format!(
            "Avalara {} error: {}",
            status, body
        )))
    } else {
        Err(TaxProviderError::Unavailable(format!(
            "Avalara {} server error: {}",
            status, body
        )))
    }
}

impl TaxProvider for AvalaraProvider {
    async fn quote_tax(&self, req: TaxQuoteRequest) -> Result<TaxQuoteResponse, TaxProviderError> {
        if req.line_items.is_empty() {
            return Err(TaxProviderError::InvalidRequest(
                "No line items provided".to_string(),
            ));
        }

        let lines: Vec<AvalaraLine> = req
            .line_items
            .iter()
            .enumerate()
            .map(|(i, li)| AvalaraLine {
                number: (i + 1).to_string(),
                amount: li.amount_minor as f64 / 100.0,
                item_code: li.tax_code.clone(),
                tax_code: li.tax_code.clone(),
                description: li.description.clone(),
                quantity: li.quantity,
            })
            .collect();

        let body = AvalaraCreateRequest {
            transaction_type: "SalesOrder".to_string(),
            code: req.invoice_id.clone(),
            company_code: self.company_code.clone(),
            date: req.invoice_date.format("%Y-%m-%d").to_string(),
            customer_code: req.customer_id.clone(),
            addresses: AvalaraAddresses {
                ship_from: map_address(&req.ship_from),
                ship_to: map_address(&req.ship_to),
            },
            lines,
            currency_code: req.currency.to_uppercase(),
            commit: None,
            reference_code: req.correlation_id.clone(),
        };

        let resp = self
            .client
            .post(format!("{}/api/v2/transactions/create", self.base_url))
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let resp = expect_avalara_success(resp).await?;

        let tx: AvalaraTransactionResponse = resp.json().await.map_err(|e| {
            TaxProviderError::Unavailable(format!("Avalara response parse error: {}", e))
        })?;

        let total_tax_minor = (tx.total_tax * 100.0).round() as i64;

        let tax_by_line: Vec<TaxByLine> = req
            .line_items
            .iter()
            .enumerate()
            .map(|(i, li)| {
                let (tax_minor, rate, jurisdiction, tax_type) = tx
                    .lines
                    .get(i)
                    .map(|l| {
                        let tax_minor = (l.tax * 100.0).round() as i64;
                        let (rate, jurisdiction, tax_type) = l
                            .details
                            .first()
                            .map(|d| (d.rate, d.tax_name.clone(), d.tax_type.clone()))
                            .unwrap_or((0.0, "unknown".to_string(), "sales_tax".to_string()));
                        (tax_minor, rate, jurisdiction, tax_type)
                    })
                    .unwrap_or((0, 0.0, "unknown".to_string(), "sales_tax".to_string()));

                TaxByLine {
                    line_id: li.line_id.clone(),
                    tax_minor,
                    rate,
                    jurisdiction,
                    tax_type,
                }
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor,
            tax_by_line,
            provider_quote_ref: tx.code,
            expires_at: None,
            quoted_at: Utc::now(),
            warnings: vec![],
        })
    }

    async fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        let url = format!(
            "{}/api/v2/companies/{}/transactions/{}/commit",
            self.base_url, self.company_code, req.provider_quote_ref
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(&AvalaraCommitRequest { commit: true })
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let resp = expect_avalara_success(resp).await?;

        let tx: AvalaraTransactionResponse = resp.json().await.map_err(|e| {
            TaxProviderError::Unavailable(format!("Avalara response parse error: {}", e))
        })?;

        Ok(TaxCommitResponse {
            provider_commit_ref: tx.code,
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(&self, req: TaxVoidRequest) -> Result<TaxVoidResponse, TaxProviderError> {
        let url = format!(
            "{}/api/v2/companies/{}/transactions/{}/void",
            self.base_url, self.company_code, req.provider_commit_ref
        );

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Accept", "application/json")
            .json(&AvalaraVoidRequest {
                code: "DocVoided".to_string(),
            })
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = resp.status();
        if status.is_success() {
            return Ok(TaxVoidResponse {
                voided: true,
                voided_at: Utc::now(),
            });
        }
        let body = resp.text().await.unwrap_or_default();
        if status.is_client_error() {
            Err(TaxProviderError::Provider(format!(
                "Avalara {} void error: {}",
                status, body
            )))
        } else {
            Err(TaxProviderError::Unavailable(format!(
                "Avalara {} server error: {}",
                status, body
            )))
        }
    }
}
