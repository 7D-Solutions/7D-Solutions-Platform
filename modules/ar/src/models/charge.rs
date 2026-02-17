use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Charge status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(rename_all = "lowercase")]
pub struct ChargeStatus(pub String);

impl ChargeStatus {
    pub const PENDING: &'static str = "pending";
    pub const SUCCEEDED: &'static str = "succeeded";
    pub const FAILED: &'static str = "failed";
    pub const AUTHORIZED: &'static str = "authorized";
    pub const CAPTURED: &'static str = "captured";
}

/// Charge record from ar_charges table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Charge {
    pub id: i32,
    pub app_id: String,
    pub tilled_charge_id: Option<String>,
    pub invoice_id: Option<i32>,
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub charge_type: String,
    pub reason: Option<String>,
    pub reference_id: Option<String>,
    pub service_date: Option<NaiveDateTime>,
    pub note: Option<String>,
    pub metadata: Option<JsonValue>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub product_type: Option<String>,
    pub quantity: Option<i32>,
    pub service_frequency: Option<String>,
    pub weight_amount: Option<String>,
    pub location_reference: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a charge
#[derive(Debug, Deserialize)]
pub struct CreateChargeRequest {
    pub ar_customer_id: i32,
    pub amount_cents: i32,
    pub currency: Option<String>,
    pub charge_type: Option<String>,
    pub reason: String,
    pub reference_id: String,
    pub service_date: Option<NaiveDateTime>,
    pub note: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Request body for capturing an authorized charge
#[derive(Debug, Deserialize)]
pub struct CaptureChargeRequest {
    pub amount_cents: Option<i32>,
}

/// Query parameters for listing charges
#[derive(Debug, Deserialize)]
pub struct ListChargesQuery {
    pub customer_id: Option<i32>,
    pub invoice_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
