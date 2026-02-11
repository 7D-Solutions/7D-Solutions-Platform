use super::error::TilledError;
use super::types::{ListResponse, PaymentMethod};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct AttachPaymentMethodRequest {
    pub customer_id: String,
}

impl TilledClient {
    /// Attach a payment method to a customer
    pub async fn attach_payment_method(
        &self,
        payment_method_id: &str,
        customer_id: String,
    ) -> Result<PaymentMethod, TilledError> {
        let path = format!("/v1/payment-methods/{}/attach", payment_method_id);
        let request = AttachPaymentMethodRequest { customer_id };

        self.post(&path, &request).await
    }

    /// Detach a payment method from a customer
    pub async fn detach_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethod, TilledError> {
        let path = format!("/v1/payment-methods/{}/detach", payment_method_id);
        // POST with empty body
        let empty: HashMap<String, String> = HashMap::new();
        self.post(&path, &empty).await
    }

    /// Get a payment method by ID
    pub async fn get_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethod, TilledError> {
        let path = format!("/v1/payment-methods/{}", payment_method_id);
        self.get(&path, None).await
    }

    /// List payment methods for a customer
    pub async fn list_payment_methods(
        &self,
        customer_id: &str,
    ) -> Result<ListResponse<PaymentMethod>, TilledError> {
        let mut params = HashMap::new();
        params.insert("customer_id".to_string(), customer_id.to_string());

        self.get("/v1/payment-methods", Some(params)).await
    }
}
