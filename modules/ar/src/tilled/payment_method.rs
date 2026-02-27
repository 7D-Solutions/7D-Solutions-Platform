use super::error::TilledError;
use super::types::{ListResponse, PaymentMethod};
use super::TilledClient;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct AttachPaymentMethodRequest {
    pub customer_id: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct CardDetailsRequest {
    pub number: String,
    pub exp_month: i32,
    pub exp_year: i32,
    pub cvv: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AddressRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct BillingDetailsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<AddressRequest>,
}

#[derive(Debug, Serialize, Clone)]
pub struct CreatePaymentMethodRequest {
    #[serde(rename = "type")]
    pub payment_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_details: Option<BillingDetailsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<CardDetailsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick_name: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct UpdatePaymentMethodRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_details: Option<BillingDetailsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nick_name: Option<String>,
}

impl TilledClient {
    /// Create a payment method (sandbox supports server-side card details for test cards).
    pub async fn create_payment_method(
        &self,
        request: CreatePaymentMethodRequest,
    ) -> Result<PaymentMethod, TilledError> {
        self.post("/v1/payment-methods", &request).await
    }

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

    /// Update a payment method.
    pub async fn update_payment_method(
        &self,
        payment_method_id: &str,
        request: UpdatePaymentMethodRequest,
    ) -> Result<PaymentMethod, TilledError> {
        let path = format!("/v1/payment-methods/{payment_method_id}");
        self.patch(&path, &request).await
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
    use super::{
        build_list_payment_methods_params, AddressRequest, BillingDetailsRequest,
        CreatePaymentMethodRequest,
    };

    #[test]
    fn list_payment_methods_query_includes_customer_and_type() {
        let params = build_list_payment_methods_params("cus_123", "card");
        assert_eq!(
            params.get("customer_id").map(String::as_str),
            Some("cus_123")
        );
        assert_eq!(params.get("type").map(String::as_str), Some("card"));
    }

    #[test]
    fn create_payment_method_payload_uses_type_field() {
        let payload = CreatePaymentMethodRequest {
            payment_type: "card".to_string(),
            billing_details: Some(BillingDetailsRequest {
                name: Some("Sandbox Test".to_string()),
                email: None,
                address: Some(AddressRequest {
                    line1: None,
                    line2: None,
                    city: None,
                    state: None,
                    postal_code: None,
                    country: Some("US".to_string()),
                    zip: Some("90210".to_string()),
                }),
            }),
            card: None,
            nick_name: None,
        };
        let json = serde_json::to_value(payload).unwrap();
        assert_eq!(json.get("type").unwrap(), "card");
    }
}
