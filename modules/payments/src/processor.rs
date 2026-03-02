use uuid::Uuid;

use crate::models::{PaymentCollectionRequestedPayload, PaymentResult};
use crate::reconciliation::PspPaymentStatus;

// ============================================================================
// TilledPaymentProcessor
// ============================================================================

/// Tilled payment processor — calls the Tilled API for real payment operations.
///
/// Requires `TILLED_API_KEY` and `TILLED_ACCOUNT_ID` in configuration.
/// Use `PAYMENTS_PROVIDER=tilled` to activate.
pub struct TilledPaymentProcessor {
    api_key: String,
    account_id: String,
    client: reqwest::Client,
}

impl TilledPaymentProcessor {
    pub fn new(api_key: String, account_id: String) -> Self {
        Self {
            api_key,
            account_id,
            client: reqwest::Client::new(),
        }
    }

    /// Process a payment via the Tilled API (payment intent flow).
    ///
    /// Tilled uses payment intents; we create a PaymentIntent and confirm it
    /// using the stored `payment_method_id`.
    pub async fn process_payment(
        &self,
        request: &PaymentCollectionRequestedPayload,
    ) -> anyhow::Result<PaymentResult> {
        tracing::info!(
            invoice_id = %request.invoice_id,
            customer_id = %request.customer_id,
            amount = request.amount_minor,
            currency = %request.currency,
            "Processing Tilled payment"
        );

        let payment_method_id = request
            .payment_method_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("payment_method_id is required for Tilled"))?;

        // Create payment intent
        let create_url = format!(
            "https://api.tilled.com/v1/payment-intents?tilled_account={}",
            self.account_id
        );
        let create_body = serde_json::json!({
            "amount": request.amount_minor,
            "currency": request.currency.to_lowercase(),
            "payment_method_types": ["card"],
            "capture_method": "automatic",
        });

        let create_resp = self
            .client
            .post(&create_url)
            .header("tilled-account", &self.account_id)
            .bearer_auth(&self.api_key)
            .json(&create_body)
            .send()
            .await?;

        if !create_resp.status().is_success() {
            let status = create_resp.status();
            let text = create_resp.text().await.unwrap_or_default();
            anyhow::bail!("Tilled create payment intent failed ({}): {}", status, text);
        }

        let pi: serde_json::Value = create_resp.json().await?;
        let pi_id = pi["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Tilled response missing payment intent id"))?;

        // Confirm payment intent with the stored payment method
        let confirm_url = format!(
            "https://api.tilled.com/v1/payment-intents/{}/confirm?tilled_account={}",
            pi_id, self.account_id
        );
        let confirm_body = serde_json::json!({
            "payment_method_id": payment_method_id,
        });

        let confirm_resp = self
            .client
            .post(&confirm_url)
            .header("tilled-account", &self.account_id)
            .bearer_auth(&self.api_key)
            .json(&confirm_body)
            .send()
            .await?;

        if !confirm_resp.status().is_success() {
            let status = confirm_resp.status();
            let text = confirm_resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Tilled confirm payment intent failed ({}): {}",
                status,
                text
            );
        }

        let confirmed: serde_json::Value = confirm_resp.json().await?;
        let pi_status = confirmed["status"].as_str().unwrap_or("unknown");

        if pi_status != "succeeded" && pi_status != "processing" {
            anyhow::bail!(
                "Tilled payment intent in unexpected status after confirm: {}",
                pi_status
            );
        }

        let payment_id = Uuid::new_v4().to_string();
        tracing::info!(
            payment_id = %payment_id,
            tilled_pi_id = pi_id,
            tilled_status = pi_status,
            "Tilled payment processed"
        );

        Ok(PaymentResult {
            payment_id,
            processor_payment_id: pi_id.to_string(),
            payment_method_ref: request.payment_method_id.clone(),
        })
    }

    /// Query Tilled for the current status of a payment intent.
    pub async fn query_payment_status(
        &self,
        processor_payment_id: &str,
    ) -> anyhow::Result<PspPaymentStatus> {
        tracing::info!(
            processor_payment_id = %processor_payment_id,
            "Querying Tilled for payment intent status"
        );

        let url = format!(
            "https://api.tilled.com/v1/payment-intents/{}?tilled_account={}",
            processor_payment_id, self.account_id
        );

        let resp = self
            .client
            .get(&url)
            .header("tilled-account", &self.account_id)
            .bearer_auth(&self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Tilled query payment intent failed ({}): {}", status, text);
        }

        let pi: serde_json::Value = resp.json().await?;
        let pi_status = pi["status"].as_str().unwrap_or("unknown");

        let psp_status = match pi_status {
            "succeeded" => PspPaymentStatus::Succeeded,
            "canceled" => PspPaymentStatus::FailedFinal {
                code: "canceled".to_string(),
                message: "Payment intent was canceled".to_string(),
            },
            "requires_payment_method" => PspPaymentStatus::FailedRetry {
                code: "requires_payment_method".to_string(),
                message: "Payment method required or failed".to_string(),
            },
            "processing" | "requires_action" | "requires_confirmation" => {
                PspPaymentStatus::StillUnknown
            }
            other => {
                tracing::warn!(
                    tilled_status = other,
                    "Unknown Tilled payment intent status"
                );
                PspPaymentStatus::StillUnknown
            }
        };

        Ok(psp_status)
    }
}

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
