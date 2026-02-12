use uuid::Uuid;

use crate::models::{PaymentCollectionRequestedPayload, PaymentResult};

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
}

impl Default for MockPaymentProcessor {
    fn default() -> Self {
        Self::new()
    }
}
