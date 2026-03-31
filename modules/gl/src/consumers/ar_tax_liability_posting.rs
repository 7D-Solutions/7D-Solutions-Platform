//! AR Tax Liability GL posting logic — pure business rules for tax journal entries
//!
//! ## Accounting Entries
//!
//! ### tax.committed (tax liability recognized at invoice finalization)
//! DR  TAX_COLLECTED    total_tax_minor / 100.0   ← tax collected from customer (clearing)
//! CR  TAX_PAYABLE      total_tax_minor / 100.0   ← liability to tax authority
//!
//! ### tax.voided (committed tax reversed on refund/write-off/cancellation)
//! DR  TAX_PAYABLE      total_tax_minor / 100.0   ← reduce liability
//! CR  TAX_COLLECTED    total_tax_minor / 100.0   ← reverse clearing balance

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::{
    Dimensions, GlPostingRequestV1, JournalLine, SourceDocType,
};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

/// Well-known GL account for tax collected from customers (clearing/offset)
pub const TAX_COLLECTED_ACCOUNT: &str = "TAX_COLLECTED";

/// Well-known GL account for sales tax payable to tax authorities (liability)
pub const TAX_PAYABLE_ACCOUNT: &str = "TAX_PAYABLE";

// ============================================================================
// Payload types
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaxCommittedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_quote_ref: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub committed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaxVoidedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub void_reason: String,
    pub voided_at: DateTime<Utc>,
}

// ============================================================================
// Posting functions (testable without NATS)
// ============================================================================

/// Process a tax.committed event and post the tax liability GL journal entry.
pub async fn process_tax_committed_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &TaxCommittedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_tax_minor as f64 / 100.0;

    let dims = Some(Dimensions {
        customer_id: Some(payload.customer_id.clone()),
        vendor_id: None,
        location_id: None,
        job_id: None,
        department: None,
        class: None,
        project: None,
    });

    let posting = GlPostingRequestV1 {
        posting_date: payload.committed_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: payload.invoice_id.clone(),
        description: format!(
            "Sales tax liability — invoice {} (provider: {})",
            payload.invoice_id, payload.provider
        ),
        lines: vec![
            JournalLine {
                account_ref: TAX_COLLECTED_ACCOUNT.to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Tax collected — invoice {} commit {}",
                    payload.invoice_id, payload.provider_commit_ref
                )),
                dimensions: dims.clone(),
            },
            JournalLine {
                account_ref: TAX_PAYABLE_ACCOUNT.to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Sales tax payable — invoice {} ({})",
                    payload.invoice_id, payload.provider
                )),
                dimensions: dims,
            },
        ],
    };

    let subject = format!("tax.committed.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}

/// Process a tax.voided event and post the reversal GL journal entry.
pub async fn process_tax_voided_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &TaxVoidedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_tax_minor as f64 / 100.0;

    let dims = Some(Dimensions {
        customer_id: Some(payload.customer_id.clone()),
        vendor_id: None,
        location_id: None,
        job_id: None,
        department: None,
        class: None,
        project: None,
    });

    let posting = GlPostingRequestV1 {
        posting_date: payload.voided_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ArAdjustment,
        source_doc_id: payload.invoice_id.clone(),
        description: format!(
            "Tax liability reversal — invoice {} ({})",
            payload.invoice_id, payload.void_reason
        ),
        lines: vec![
            JournalLine {
                account_ref: TAX_PAYABLE_ACCOUNT.to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Tax payable reversal — invoice {} ({})",
                    payload.invoice_id, payload.void_reason
                )),
                dimensions: dims.clone(),
            },
            JournalLine {
                account_ref: TAX_COLLECTED_ACCOUNT.to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Tax collected reversal — invoice {} void {}",
                    payload.invoice_id, payload.provider_commit_ref
                )),
                dimensions: dims,
            },
        ],
    };

    let subject = format!("tax.voided.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}
