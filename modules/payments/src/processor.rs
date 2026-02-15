use uuid::Uuid;

use crate::models::{PaymentCollectionRequestedPayload, PaymentResult};
use crate::reconciliation::PspPaymentStatus;

/// Mock payment processor for development and testing
///
/// In production, this would be replaced with actual processor integrations
/// (Stripe, Tilled, etc.). For now, it simulates successful payment processing.
pub struct MockPaymentProcessor;

impl MockPaymentProcessor {
    pub fn new() -> Self {
        Self
    }

    /// Process a payment request
    ///
    /// Mock implementation that simulates payment processing.
    /// - If payment_method_id starts with "fail_", the payment will fail
    /// - Otherwise, the payment succeeds
    ///
    /// In production, this would call external payment processor APIs.
    pub async fn process_payment(
        &self,
        request: &PaymentCollectionRequestedPayload,
    ) -> anyhow::Result<PaymentResult> {
        tracing::info!(
            invoice_id = %request.invoice_id,
            customer_id = %request.customer_id,
            amount = request.amount_minor,
            currency = %request.currency,
            payment_method_id = ?request.payment_method_id,
            "Processing mock payment"
        );

        // Simulate processing delay
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Check for failure simulation trigger
        if let Some(ref payment_method_id) = request.payment_method_id {
            if payment_method_id.starts_with("fail_") {
                tracing::warn!(
                    invoice_id = %request.invoice_id,
                    payment_method_id = %payment_method_id,
                    "Mock payment failed (triggered by payment_method_id)"
                );
                return Err(anyhow::anyhow!(
                    "Payment declined by processor: insufficient funds"
                ));
            }
        }

        // Generate mock payment IDs
        let payment_id = Uuid::new_v4().to_string();
        let processor_payment_id = format!("mock_pi_{}", Uuid::new_v4().simple());

        tracing::info!(
            payment_id = %payment_id,
            processor_payment_id = %processor_payment_id,
            "Mock payment processed successfully"
        );

        Ok(PaymentResult {
            payment_id,
            processor_payment_id,
            payment_method_ref: request.payment_method_id.clone(),
        })
    }

    /// Query payment status from PSP (for UNKNOWN reconciliation)
    ///
    /// **Mock Implementation for bd-2uw:**
    /// - Processor ID contains "unknown_" → StillUnknown
    /// - Processor ID contains "succeeded_" → Succeeded
    /// - Processor ID contains "failed_retry_" → FailedRetry
    /// - Processor ID contains "failed_final_" → FailedFinal
    /// - Default → Succeeded (assume success if no special marker)
    ///
    /// **Production Implementation:**
    /// - Query actual PSP API (Stripe, Tilled, etc.)
    /// - Handle PSP-specific error codes
    /// - Map PSP status to PspPaymentStatus enum
    ///
    /// **Usage:**
    /// ```ignore
    /// let processor = MockPaymentProcessor::new();
    /// let status = processor.query_payment_status("mock_pi_12345").await?;
    /// match status {
    ///     PspPaymentStatus::Succeeded => { /* handle success */ }
    ///     PspPaymentStatus::FailedRetry { .. } => { /* handle transient failure */ }
    ///     PspPaymentStatus::FailedFinal { .. } => { /* handle permanent failure */ }
    ///     PspPaymentStatus::StillUnknown => { /* defer reconciliation */ }
    /// }
    /// ```
    pub async fn query_payment_status(
        &self,
        processor_payment_id: &str,
    ) -> anyhow::Result<PspPaymentStatus> {
        tracing::info!(
            processor_payment_id = %processor_payment_id,
            "Querying PSP for payment status (mock implementation)"
        );

        // Simulate PSP query delay
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Mock logic based on processor_payment_id markers
        let status = if processor_payment_id.contains("unknown_") {
            tracing::warn!(
                processor_payment_id = %processor_payment_id,
                "PSP still does not know payment status"
            );
            PspPaymentStatus::StillUnknown
        } else if processor_payment_id.contains("succeeded_") {
            tracing::info!(
                processor_payment_id = %processor_payment_id,
                "PSP confirms payment succeeded"
            );
            PspPaymentStatus::Succeeded
        } else if processor_payment_id.contains("failed_retry_") {
            tracing::warn!(
                processor_payment_id = %processor_payment_id,
                "PSP reports transient payment failure"
            );
            PspPaymentStatus::FailedRetry {
                code: "insufficient_funds".to_string(),
                message: "Insufficient funds (transient)".to_string(),
            }
        } else if processor_payment_id.contains("failed_final_") {
            tracing::warn!(
                processor_payment_id = %processor_payment_id,
                "PSP reports permanent payment failure"
            );
            PspPaymentStatus::FailedFinal {
                code: "card_declined".to_string(),
                message: "Card declined (permanent)".to_string(),
            }
        } else {
            // Default: assume payment succeeded (most common case)
            tracing::info!(
                processor_payment_id = %processor_payment_id,
                "PSP query returned success (default)"
            );
            PspPaymentStatus::Succeeded
        };

        Ok(status)
    }
}

impl Default for MockPaymentProcessor {
    fn default() -> Self {
        Self::new()
    }
}
