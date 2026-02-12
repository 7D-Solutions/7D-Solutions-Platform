//! GL Posting Request V1 Contract Types
//!
//! These types match the JSON schema defined in:
//! contracts/events/gl-posting-request.v1.json
//!
//! IMPORTANT: Field names must match the JSON schema EXACTLY (case-sensitive).
//! Do not add validations beyond what's in the schema.

use serde::{Deserialize, Serialize};

/// Payload for GL posting request event
///
/// This is the payload type used with `EventEnvelope<GlPostingRequestV1>`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlPostingRequestV1 {
    /// Accounting date for the journal entry (YYYY-MM-DD)
    pub posting_date: String,

    /// ISO 4217 currency code (e.g., "USD", "EUR")
    pub currency: String,

    /// Document type that originated this posting
    pub source_doc_type: SourceDocType,

    /// Unique identifier of the source document in the originating module
    pub source_doc_id: String,

    /// Human-readable description for the journal entry (1-500 chars)
    pub description: String,

    /// Journal entry lines (must have at least 2 items)
    pub lines: Vec<JournalLine>,
}

/// Source document types that can generate GL postings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SourceDocType {
    ArInvoice,
    ArPayment,
    ArCreditMemo,
    ArAdjustment,
    ApBill,
    ApPayment,
    InventoryReceipt,
    InventoryIssue,
    PayrollRun,
}

/// A single line in a journal entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalLine {
    /// Reference to account in the GL chart of accounts
    pub account_ref: String,

    /// Debit amount (must be >= 0)
    pub debit: f64,

    /// Credit amount (must be >= 0)
    pub credit: f64,

    /// Optional line-level memo (<= 500 chars)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,

    /// Optional analytical dimensions for reporting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<Dimensions>,
}

/// Analytical dimensions for reporting and analysis
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Dimensions {
    /// Customer identifier for AR-related postings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<String>,

    /// Vendor identifier for AP-related postings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,

    /// Location or branch identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_id: Option<String>,

    /// Job or project work order identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,

    /// Department code
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,

    /// Classification dimension
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,

    /// Project identifier for project accounting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_deserialize_valid_payload() {
        let json = r#"{
            "posting_date": "2024-02-11",
            "currency": "USD",
            "source_doc_type": "AR_INVOICE",
            "source_doc_id": "inv_01HPQW9K7J4M6N8P2R5T7V9W1X",
            "description": "Invoice for customer services",
            "lines": [
                {
                    "account_ref": "1100",
                    "debit": 2599.00,
                    "credit": 0,
                    "memo": "Accounts Receivable",
                    "dimensions": {
                        "customer_id": "cus_01HPQW8Z5N7P9Q2R4T6V8W1X3Y"
                    }
                },
                {
                    "account_ref": "4000",
                    "debit": 0,
                    "credit": 2599.00,
                    "memo": "Revenue"
                }
            ]
        }"#;

        let result: Result<GlPostingRequestV1, _> = serde_json::from_str(json);
        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload.posting_date, "2024-02-11");
        assert_eq!(payload.currency, "USD");
        assert_eq!(payload.source_doc_type, SourceDocType::ArInvoice);
        assert_eq!(payload.lines.len(), 2);
        assert_eq!(payload.lines[0].debit, 2599.00);
        assert_eq!(payload.lines[1].credit, 2599.00);
    }

    #[test]
    fn test_deserialize_minimal_payload() {
        let json = r#"{
            "posting_date": "2024-02-11",
            "currency": "USD",
            "source_doc_type": "AR_PAYMENT",
            "source_doc_id": "pay_123",
            "description": "Payment received",
            "lines": [
                {
                    "account_ref": "1000",
                    "debit": 100.00,
                    "credit": 0
                },
                {
                    "account_ref": "1100",
                    "debit": 0,
                    "credit": 100.00
                }
            ]
        }"#;

        let result: Result<GlPostingRequestV1, _> = serde_json::from_str(json);
        assert!(result.is_ok());

        let payload = result.unwrap();
        assert_eq!(payload.lines[0].memo, None);
        assert_eq!(payload.lines[0].dimensions, None);
    }

    #[test]
    fn test_source_doc_type_variants() {
        let types = vec![
            ("AR_INVOICE", SourceDocType::ArInvoice),
            ("AR_PAYMENT", SourceDocType::ArPayment),
            ("AR_CREDIT_MEMO", SourceDocType::ArCreditMemo),
            ("AR_ADJUSTMENT", SourceDocType::ArAdjustment),
            ("AP_BILL", SourceDocType::ApBill),
            ("AP_PAYMENT", SourceDocType::ApPayment),
            ("INVENTORY_RECEIPT", SourceDocType::InventoryReceipt),
            ("INVENTORY_ISSUE", SourceDocType::InventoryIssue),
            ("PAYROLL_RUN", SourceDocType::PayrollRun),
        ];

        for (json_val, expected) in types {
            let json = format!(r#"{{"source_doc_type": "{}"}}"#, json_val);
            let result: serde_json::Result<serde_json::Value> = serde_json::from_str(&json);
            assert!(result.is_ok());

            // Verify it serializes to the correct JSON string
            let serialized = serde_json::to_string(&expected).unwrap();
            assert_eq!(serialized, format!(r#""{}""#, json_val));
        }
    }
}
