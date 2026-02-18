//! GL HTTP client — fetches trial balance and period close data from the GL service.
//!
//! The consolidation module calls GL's existing APIs rather than directly
//! accessing GL's database, preserving module boundaries.

use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum GlClientError {
    #[error("GL API request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("GL API returned {status}: {body}")]
    Api { status: u16, body: String },

    #[error("Period {0} is not closed for tenant {1}")]
    PeriodNotClosed(Uuid, String),

    #[error("Hash mismatch for entity {entity}: expected {expected}, got {actual}")]
    HashMismatch {
        entity: String,
        expected: String,
        actual: String,
    },
}

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
    CloseRequested { requested_at: String },
    Closed {
        closed_at: String,
        closed_by: String,
        close_reason: Option<String>,
        close_hash: String,
        requested_at: Option<String>,
    },
}

/// HTTP client for the GL service.
#[derive(Clone)]
pub struct GlClient {
    client: Client,
    base_url: String,
}

impl GlClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetch trial balance for an entity (tenant) + period + currency.
    pub async fn get_trial_balance(
        &self,
        tenant_id: &str,
        period_id: Uuid,
        currency: &str,
    ) -> Result<GlTrialBalanceResponse, GlClientError> {
        let url = format!("{}/api/gl/trial-balance", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("tenant_id", tenant_id),
                ("period_id", &period_id.to_string()),
                ("currency", currency),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GlClientError::Api { status, body });
        }

        Ok(resp.json().await?)
    }

    /// Fetch period close status to get the close_hash for verification.
    pub async fn get_close_status(
        &self,
        tenant_id: &str,
        period_id: Uuid,
    ) -> Result<GlCloseStatusResponse, GlClientError> {
        let url = format!("{}/api/gl/periods/{}/close-status", self.base_url, period_id);
        let resp = self
            .client
            .get(&url)
            .query(&[("tenant_id", tenant_id)])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(GlClientError::Api { status, body });
        }

        Ok(resp.json().await?)
    }

    /// Fetch close hash for a period. Returns None if the period is not closed.
    pub async fn get_close_hash(
        &self,
        tenant_id: &str,
        period_id: Uuid,
    ) -> Result<Option<String>, GlClientError> {
        let status = self.get_close_status(tenant_id, period_id).await?;
        match status.close_status {
            GlCloseStatus::Closed { close_hash, .. } => Ok(Some(close_hash)),
            _ => Ok(None),
        }
    }

    /// Post an elimination journal entry to GL.
    ///
    /// Uses source_module = "consolidation-elimination" for audit trail.
    /// The source_doc_id acts as an idempotency reference within GL.
    /// Returns the journal entry ID from GL.
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
    ) -> Result<Uuid, GlClientError> {
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

        let url = format!("{}/api/gl/journal-entries", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(GlClientError::Api {
                status,
                body: body_text,
            });
        }

        let result: PostJournalResponse = resp.json().await?;
        Ok(result.journal_entry_id)
    }
}

/// Response from GL journal entry creation endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct PostJournalResponse {
    pub journal_entry_id: Uuid,
}
