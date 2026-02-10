use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Common metadata type
pub type Metadata = HashMap<String, String>;

/// Customer response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub metadata: Option<Metadata>,
    pub created_at: Option<i64>,
}

/// Payment method response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethod {
    pub id: String,
    pub customer_id: Option<String>,
    #[serde(rename = "type")]
    pub payment_type: String,
    pub card: Option<CardDetails>,
    pub billing_details: Option<BillingDetails>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetails {
    pub brand: String,
    pub last4: String,
    pub exp_month: i32,
    pub exp_year: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingDetails {
    pub name: Option<String>,
    pub email: Option<String>,
    pub address: Option<Address>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub line1: Option<String>,
    pub line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// Payment intent response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub customer_id: Option<String>,
    pub payment_method_id: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<Metadata>,
    pub last_payment_error: Option<PaymentError>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentError {
    pub code: String,
    pub message: String,
}

/// Subscription response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    pub customer_id: String,
    pub payment_method_id: String,
    pub status: String,
    pub price: i64,
    pub currency: String,
    pub interval_unit: String,
    pub interval_count: i32,
    pub billing_cycle_anchor: Option<i64>,
    pub trial_end: Option<i64>,
    pub cancel_at_period_end: bool,
    pub metadata: Option<Metadata>,
    pub created_at: Option<i64>,
    pub canceled_at: Option<i64>,
}

/// Refund response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refund {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub payment_intent_id: Option<String>,
    pub charge_id: Option<String>,
    pub reason: Option<String>,
    pub metadata: Option<Metadata>,
    pub created_at: Option<i64>,
}

/// Dispute response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dispute {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub payment_intent_id: Option<String>,
    pub reason: String,
    pub created_at: Option<i64>,
}

/// List response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse<T> {
    pub data: Vec<T>,
    pub has_more: bool,
}
