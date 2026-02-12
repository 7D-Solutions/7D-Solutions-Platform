use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Subscription Plan
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SubscriptionPlan {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub schedule: String,
    pub price_minor: i64,
    pub currency: String,
    pub proration_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionPlanRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub schedule: String,
    pub price_minor: i64,
    pub currency: String,
    #[serde(default)]
    pub proration_enabled: bool,
}

// ============================================================================
// Subscription
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Subscription {
    pub id: Uuid,
    pub tenant_id: String,
    pub ar_customer_id: String,
    pub plan_id: Uuid,
    pub status: String,
    pub schedule: String,
    pub price_minor: i64,
    pub currency: String,
    pub start_date: NaiveDate,
    pub next_bill_date: NaiveDate,
    pub paused_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSubscriptionRequest {
    pub tenant_id: String,
    pub ar_customer_id: String,
    pub plan_id: Uuid,
    pub schedule: String,
    pub start_date: NaiveDate,
    pub price_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseSubscriptionRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelSubscriptionRequest {
    pub reason: Option<String>,
    #[serde(default)]
    pub immediate: bool,
}

// ============================================================================
// Bill Run
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillRun {
    pub id: Uuid,
    pub bill_run_id: String,
    pub execution_date: NaiveDate,
    pub subscriptions_processed: i32,
    pub invoices_created: i32,
    pub failures: i32,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteBillRunRequest {
    pub bill_run_id: Option<String>,
    pub execution_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillRunResult {
    pub bill_run_id: String,
    pub subscriptions_processed: i32,
    pub invoices_created: i32,
    pub failures: i32,
    pub execution_time: DateTime<Utc>,
}

// ============================================================================
// AR API Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub customer_id: String,
    pub amount_due: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    pub customer_id: String,
    pub status: String,
    pub amount_due: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalizeInvoiceRequest {
    pub auto_advance: Option<bool>,
}

// ============================================================================
// Events
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillRunCompletedPayload {
    pub bill_run_id: String,
    pub subscriptions_processed: i32,
    pub invoices_created: i32,
    pub failures: i32,
    pub execution_time: DateTime<Utc>,
}

// ============================================================================
// Error
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}
