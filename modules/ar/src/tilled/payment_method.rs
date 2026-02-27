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
        let url = format!("{}{}", self.config().base_path, path);
        let response = self
            .http_client
            .put(&url)
            .headers(self.build_auth_headers()?)
            .json(&request)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;
        self.handle_response(response).await
    }

    /// Detach a payment method from a customer
    pub async fn detach_payment_method(
        &self,
        payment_method_id: &str,
    ) -> Result<PaymentMethod, TilledError> {
        let path = format!("/v1/payment-methods/{}/detach", payment_method_id);
        // PUT with empty body
        let empty: HashMap<String, String> = HashMap::new();
        let url = format!("{}{}", self.config().base_path, path);
        let response = self
            .http_client
            .put(&url)
            .headers(self.build_auth_headers()?)
            .json(&empty)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;
        self.handle_response(response).await
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
        pm_type: &str,
    ) -> Result<ListResponse<PaymentMethod>, TilledError> {
        let params = build_list_payment_methods_params(customer_id, pm_type);

        self.get("/v1/payment-methods", Some(params)).await
    }
}

fn build_list_payment_methods_params(customer_id: &str, pm_type: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    params.insert("customer_id".to_string(), customer_id.to_string());
    params.insert("type".to_string(), pm_type.to_string());
    params
}

#[cfg(test)]
mod tests {
    use super::build_list_payment_methods_params;

    #[test]
    fn list_payment_methods_query_includes_customer_and_type() {
        let params = build_list_payment_methods_params("cus_123", "card");
        assert_eq!(
            params.get("customer_id").map(String::as_str),
            Some("cus_123")
        );
        assert_eq!(params.get("type").map(String::as_str), Some("card"));
    }
}
