use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

/// Customer record from ar_customers table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Customer {
    pub id: i32,
    pub app_id: String,
    pub external_customer_id: Option<String>,
    pub tilled_customer_id: Option<String>,
    pub status: String,
    pub email: String,
    pub name: Option<String>,
    pub default_payment_method_id: Option<String>,
    pub payment_method_type: Option<String>,
    pub metadata: Option<JsonValue>,
    pub update_source: Option<String>,
    pub updated_by: Option<String>,
    pub delinquent_since: Option<NaiveDateTime>,
    pub grace_period_end: Option<NaiveDateTime>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub retry_attempt_count: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// Optional link to a Party record in the party-master service.
    #[sqlx(default)]
    pub party_id: Option<Uuid>,
}

/// Request body for creating a customer
#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub external_customer_id: Option<String>,
    pub metadata: Option<JsonValue>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

/// Request body for updating a customer
#[derive(Debug, Deserialize)]
pub struct UpdateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub metadata: Option<JsonValue>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

/// Query parameters for listing customers
#[derive(Debug, Deserialize)]
pub struct ListCustomersQuery {
    pub external_customer_id: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_customer_request_party_id_is_optional() {
        let req = CreateCustomerRequest {
            email: Some("test@example.com".to_string()),
            name: None,
            external_customer_id: None,
            metadata: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
    }

    #[test]
    fn create_customer_request_accepts_party_id() {
        let id = Uuid::new_v4();
        let req = CreateCustomerRequest {
            email: Some("test@example.com".to_string()),
            name: None,
            external_customer_id: None,
            metadata: None,
            party_id: Some(id),
        };
        assert_eq!(req.party_id, Some(id));
    }

    #[test]
    fn update_customer_request_party_id_is_optional() {
        let req = UpdateCustomerRequest {
            email: None,
            name: None,
            metadata: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
    }
}
