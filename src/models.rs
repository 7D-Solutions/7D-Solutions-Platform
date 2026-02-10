use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Customer record from billing_customers table
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
}

/// Request body for creating a customer
#[derive(Debug, Deserialize)]
pub struct CreateCustomerRequest {
    pub email: String,
    pub name: Option<String>,
    pub external_customer_id: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Request body for updating a customer
#[derive(Debug, Deserialize)]
pub struct UpdateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub metadata: Option<JsonValue>,
}

/// Query parameters for listing customers
#[derive(Debug, Deserialize)]
pub struct ListCustomersQuery {
    pub external_customer_id: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Standard error response
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

/// Subscription status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billing_subscriptions_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Incomplete,
    IncompleteExpired,
    Trialing,
    Active,
    PastDue,
    Canceled,
    Unpaid,
    Paused,
}

/// Subscription interval enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billing_subscriptions_interval", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionInterval {
    Day,
    Week,
    Month,
    Year,
}

/// Subscription record from billing_subscriptions table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: i32,
    pub app_id: String,
    pub billing_customer_id: i32,
    pub tilled_subscription_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub price_cents: i32,
    pub status: SubscriptionStatus,
    pub interval_unit: SubscriptionInterval,
    pub interval_count: i32,
    pub billing_cycle_anchor: Option<NaiveDateTime>,
    pub current_period_start: NaiveDateTime,
    pub current_period_end: NaiveDateTime,
    pub cancel_at_period_end: bool,
    pub cancel_at: Option<NaiveDateTime>,
    pub canceled_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
    pub payment_method_id: String,
    pub payment_method_type: String,
    pub metadata: Option<JsonValue>,
    pub update_source: Option<String>,
    pub updated_by: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a subscription
#[derive(Debug, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub billing_customer_id: i32,
    pub payment_method_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub price_cents: i32,
    pub interval_unit: Option<SubscriptionInterval>,
    pub interval_count: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Request body for updating a subscription
#[derive(Debug, Deserialize)]
pub struct UpdateSubscriptionRequest {
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub price_cents: Option<i32>,
    pub metadata: Option<JsonValue>,
}

/// Request body for canceling a subscription
#[derive(Debug, Deserialize)]
pub struct CancelSubscriptionRequest {
    pub cancel_at_period_end: Option<bool>,
}

/// Query parameters for listing subscriptions
#[derive(Debug, Deserialize)]
pub struct ListSubscriptionsQuery {
    pub customer_id: Option<i32>,
    pub status: Option<SubscriptionStatus>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

// ============================================================================
// INVOICE MODELS
// ============================================================================

/// Invoice status enum
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
#[serde(rename_all = "lowercase")]
pub struct InvoiceStatus(pub String);

impl InvoiceStatus {
    pub const DRAFT: &'static str = "draft";
    pub const OPEN: &'static str = "open";
    pub const PAID: &'static str = "paid";
    pub const VOID: &'static str = "void";
    pub const UNCOLLECTIBLE: &'static str = "uncollectible";
}

/// Invoice record from billing_invoices table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Invoice {
    pub id: i32,
    pub app_id: String,
    pub tilled_invoice_id: String,
    pub billing_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
    pub hosted_url: Option<String>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<JsonValue>,
    pub compliance_codes: Option<JsonValue>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating an invoice
#[derive(Debug, Deserialize)]
pub struct CreateInvoiceRequest {
    pub billing_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub amount_cents: i32,
    pub currency: Option<String>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<JsonValue>,
    pub compliance_codes: Option<JsonValue>,
}

/// Request body for updating an invoice
#[derive(Debug, Deserialize)]
pub struct UpdateInvoiceRequest {
    pub status: Option<String>,
    pub amount_cents: Option<i32>,
    pub due_at: Option<NaiveDateTime>,
    pub metadata: Option<JsonValue>,
}

/// Request body for finalizing an invoice
#[derive(Debug, Deserialize)]
pub struct FinalizeInvoiceRequest {
    pub paid_at: Option<NaiveDateTime>,
}

/// Query parameters for listing invoices
#[derive(Debug, Deserialize)]
pub struct ListInvoicesQuery {
    pub customer_id: Option<i32>,
    pub subscription_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

// ============================================================================
// CHARGE MODELS
// ============================================================================

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

/// Charge record from billing_charges table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Charge {
    pub id: i32,
    pub app_id: String,
    pub tilled_charge_id: Option<String>,
    pub invoice_id: Option<i32>,
    pub billing_customer_id: i32,
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
    pub weight_amount: Option<String>, // Decimal stored as string
    pub location_reference: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for creating a charge
#[derive(Debug, Deserialize)]
pub struct CreateChargeRequest {
    pub billing_customer_id: i32,
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

// ============================================================================
// PAYMENT METHOD MODELS
// ============================================================================

/// Payment method record from billing_payment_methods table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaymentMethod {
    pub id: i32,
    pub app_id: String,
    pub billing_customer_id: i32,
    pub tilled_payment_method_id: String,
    pub status: String, // active, pending, failed
    #[serde(rename = "type")]
    pub payment_type: String, // card, ach_debit, etc.
    pub brand: Option<String>,       // visa, mastercard, amex
    pub last4: Option<String>,       // last 4 digits
    pub exp_month: Option<i32>,
    pub exp_year: Option<i32>,
    pub bank_name: Option<String>,   // for ACH
    pub bank_last4: Option<String>,  // for ACH
    pub is_default: bool,
    pub metadata: Option<JsonValue>,
    pub deleted_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for adding a payment method
#[derive(Debug, Deserialize)]
pub struct AddPaymentMethodRequest {
    pub billing_customer_id: i32,
    pub tilled_payment_method_id: String,
}

/// Request body for updating a payment method
#[derive(Debug, Deserialize)]
pub struct UpdatePaymentMethodRequest {
    pub metadata: Option<JsonValue>,
}

/// Request body for setting default payment method
#[derive(Debug, Deserialize)]
pub struct SetDefaultPaymentMethodRequest {
    pub tilled_payment_method_id: String,
}

/// Query parameters for listing payment methods
#[derive(Debug, Deserialize)]
pub struct ListPaymentMethodsQuery {
    pub customer_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

// ============================================================================
// WEBHOOK MODELS
// ============================================================================

/// Webhook status enum (matches billing_webhooks_status in schema)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "billing_webhooks_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum WebhookStatus {
    Received,
    Processing,
    Processed,
    Failed,
}

/// Webhook record from billing_webhooks table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: i32,
    pub app_id: String,
    pub event_id: String,
    pub event_type: String,
    pub status: WebhookStatus,
    pub error: Option<String>,
    pub payload: Option<JsonValue>,
    pub attempt_count: i32,
    pub last_attempt_at: Option<NaiveDateTime>,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub dead_at: Option<NaiveDateTime>,
    pub error_code: Option<String>,
    pub received_at: NaiveDateTime,
    pub processed_at: Option<NaiveDateTime>,
}

/// Webhook attempt record from billing_webhook_attempts table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookAttempt {
    pub id: i32,
    pub app_id: String,
    pub event_id: String,
    pub attempt_number: i32,
    pub status: String,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Incoming Tilled webhook event payload
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TilledWebhookEvent {
    pub id: String,              // Event ID (for idempotency)
    #[serde(rename = "type")]
    pub event_type: String,      // e.g., "payment_intent.succeeded"
    pub data: JsonValue,         // Event data object
    pub created_at: i64,         // Unix timestamp
    pub livemode: Option<bool>,  // Test vs live mode
}

/// Request to replay a failed webhook
#[derive(Debug, Deserialize)]
pub struct ReplayWebhookRequest {
    pub force: Option<bool>, // Force replay even if not failed
}

/// Query parameters for listing webhooks
#[derive(Debug, Deserialize)]
pub struct ListWebhooksQuery {
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
