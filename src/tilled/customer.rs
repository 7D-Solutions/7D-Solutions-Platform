use super::error::TilledError;
use super::types::{Customer, Metadata};
use super::TilledClient;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CreateCustomerRequest {
    pub email: String,
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Serialize)]
pub struct UpdateCustomerRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

impl TilledClient {
    /// Create a new customer
    pub async fn create_customer(
        &self,
        email: String,
        name: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<Customer, TilledError> {
        let request = CreateCustomerRequest {
            email,
            first_name: name,
            last_name: None,
            metadata,
        };

        self.post("/v1/customers", &request).await
    }

    /// Get a customer by ID
    pub async fn get_customer(&self, customer_id: &str) -> Result<Customer, TilledError> {
        let path = format!("/v1/customers/{}", customer_id);
        self.get(&path, None).await
    }

    /// Update a customer
    pub async fn update_customer(
        &self,
        customer_id: &str,
        updates: UpdateCustomerRequest,
    ) -> Result<Customer, TilledError> {
        let path = format!("/v1/customers/{}", customer_id);
        self.patch(&path, &updates).await
    }

    /// Delete a customer
    pub async fn delete_customer(&self, customer_id: &str) -> Result<Customer, TilledError> {
        let path = format!("/v1/customers/{}", customer_id);
        self.delete(&path).await
    }
}
