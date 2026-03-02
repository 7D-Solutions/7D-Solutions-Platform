//! Payments disbursement integration layer.
//!
//! This module is the seam between AP payment runs and the underlying
//! disbursement service (ACH, wire, check processor).
//!
//! In production this would make an HTTP call to the Payments module.
//! The integration is synchronous from AP's perspective: AP submits an
//! instruction and gets back a stable `payment_id` for reconciliation.
//!
//! ## Idempotency
//!
//! `submit_payment` derives `payment_id` deterministically from `run_id +
//! vendor_id` using UUID v5. Re-submitting the same instruction for the same
//! run + vendor always returns the same `payment_id`, making it safe to retry
//! the execute endpoint without risk of double-payment.

use chrono::{DateTime, Utc};
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

/// A payment instruction to be submitted to the disbursement service.
pub struct PaymentInstruction {
    /// AP payment run that owns this instruction.
    pub run_id: Uuid,
    /// Vendor receiving the payment.
    pub vendor_id: Uuid,
    /// Total to disburse in minor currency units.
    pub amount_minor: i64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// "ach", "wire", or "check"
    pub payment_method: String,
    /// Tenant context (for multi-tenant Payments module routing).
    pub tenant_id: String,
}

/// Result returned by the disbursement service after submitting an instruction.
pub struct PaymentResult {
    /// Stable identifier assigned by the disbursement service.
    /// Derived deterministically from run_id + vendor_id (UUID v5) so that
    /// AP can correlate future callbacks without an external round-trip.
    pub payment_id: Uuid,
    /// When the payment instruction was accepted.
    pub executed_at: DateTime<Utc>,
    /// Optional reference from the bank or processor (e.g. ACH trace number).
    pub bank_reference: Option<String>,
}

// ============================================================================
// Public API
// ============================================================================

/// Submit a payment instruction to the disbursement module.
///
/// Returns a `PaymentResult` with a deterministic `payment_id`.
///
/// **Integration note:** Replace the body of this function with an HTTP call
/// to the Payments disbursement service when that service is available.
/// The UUID v5 derivation here ensures the same `payment_id` is returned on
/// retries, which is critical for idempotent allocation recording.
pub fn submit_payment(instruction: &PaymentInstruction) -> PaymentResult {
    // Derive a stable payment_id from run_id + vendor_id.
    // This is the integration seam: a real implementation would call the
    // Payments disbursement API here and return its assigned payment_id.
    let key = format!("{}:{}", instruction.run_id, instruction.vendor_id);
    let payment_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes());

    PaymentResult {
        payment_id,
        executed_at: Utc::now(),
        bank_reference: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_instruction() -> PaymentInstruction {
        PaymentInstruction {
            run_id: Uuid::new_v4(),
            vendor_id: Uuid::new_v4(),
            amount_minor: 50000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            tenant_id: "tenant-1".to_string(),
        }
    }

    #[test]
    fn payment_id_is_deterministic_for_same_run_and_vendor() {
        let instr = sample_instruction();
        let r1 = submit_payment(&instr);
        let r2 = submit_payment(&instr);
        assert_eq!(
            r1.payment_id, r2.payment_id,
            "same instruction → same payment_id"
        );
    }

    #[test]
    fn different_vendors_produce_different_payment_ids() {
        let run_id = Uuid::new_v4();
        let instr1 = PaymentInstruction {
            run_id,
            vendor_id: Uuid::new_v4(),
            amount_minor: 10000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            tenant_id: "tenant-1".to_string(),
        };
        let instr2 = PaymentInstruction {
            run_id,
            vendor_id: Uuid::new_v4(),
            amount_minor: 10000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            tenant_id: "tenant-1".to_string(),
        };
        let r1 = submit_payment(&instr1);
        let r2 = submit_payment(&instr2);
        assert_ne!(r1.payment_id, r2.payment_id);
    }

    #[test]
    fn different_runs_produce_different_payment_ids() {
        let vendor_id = Uuid::new_v4();
        let r1 = submit_payment(&PaymentInstruction {
            run_id: Uuid::new_v4(),
            vendor_id,
            amount_minor: 10000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            tenant_id: "tenant-1".to_string(),
        });
        let r2 = submit_payment(&PaymentInstruction {
            run_id: Uuid::new_v4(),
            vendor_id,
            amount_minor: 10000,
            currency: "USD".to_string(),
            payment_method: "ach".to_string(),
            tenant_id: "tenant-1".to_string(),
        });
        assert_ne!(r1.payment_id, r2.payment_id);
    }
}
