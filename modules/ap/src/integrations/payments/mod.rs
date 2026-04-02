//! Payment run execution — stable payment identity for AP.
//!
//! AP payment runs are **self-contained**: execution assigns a deterministic
//! `payment_id` used for idempotent allocation recording and reconciliation
//! **within AP**. This is **not** an HTTP integration with the Payments module.
//!
//! The API is synchronous: given a [`PaymentInstruction`], returns a
//! [`PaymentResult`] with the same `payment_id` for the same run + vendor
//! (UUID v5), so retries do not double-book.
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

/// Assign a deterministic `payment_id` for this payment instruction.
///
/// Used by payment run execution so re-execution yields the same idempotency
/// key (run + vendor). **Does not** call the Payments module or any external PSP.
pub fn submit_payment(instruction: &PaymentInstruction) -> PaymentResult {
    // Stable id for allocation rows and outbox correlation (AP-internal).
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
