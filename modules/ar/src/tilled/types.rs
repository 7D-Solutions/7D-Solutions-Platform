use super::error::TilledError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Common metadata type
pub type Metadata = HashMap<String, String>;

/// Customer response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Payment method response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentMethod {
    pub id: String,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(rename = "type")]
    pub payment_type: String,
    #[serde(default)]
    pub card: Option<CardDetails>,
    #[serde(default)]
    pub billing_details: Option<BillingDetails>,
    #[serde(default)]
    pub created_at: Option<String>,
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
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub payment_method_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub last_payment_error: Option<PaymentError>,
    #[serde(default)]
    pub charges: Vec<Charge>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Individual charge within a payment intent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charge {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub amount: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentError {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Subscription response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    #[serde(default)]
    pub customer_id: Option<String>,
    #[serde(default)]
    pub payment_method_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub price: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub interval_unit: Option<String>,
    #[serde(default)]
    pub interval_count: Option<i32>,
    #[serde(default)]
    pub billing_cycle_anchor: Option<String>,
    #[serde(default)]
    pub trial_end: Option<String>,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub canceled_at: Option<String>,
}

/// Refund response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refund {
    pub id: String,
    pub amount: i64,
    #[serde(default)]
    pub currency: Option<String>,
    pub status: String,
    #[serde(default)]
    pub payment_intent_id: Option<String>,
    #[serde(default)]
    pub charge_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Dispute response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dispute {
    pub id: String,
    #[serde(default)]
    pub amount: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    pub status: String,
    /// Tilled returns `charge_id`; alias for legacy code that used `payment_intent_id`.
    #[serde(default, alias = "payment_intent_id")]
    pub charge_id: Option<String>,
    #[serde(default, alias = "reason")]
    pub reason_description: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Connected account response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectedAccount {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    pub status: String,
    #[serde(default)]
    pub capabilities: Option<serde_json::Value>,
    #[serde(default)]
    pub bank_accounts: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub metadata: Option<Metadata>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Payout response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payout {
    pub id: String,
    #[serde(default)]
    pub amount: Option<i64>,
    #[serde(default)]
    pub currency: Option<String>,
    pub status: String,
    #[serde(default)]
    pub arrival_date: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Balance transaction response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceTransaction {
    pub id: String,
    pub amount: f64,
    #[serde(default)]
    pub fee: Option<f64>,
    #[serde(default)]
    pub net: Option<f64>,
    #[serde(default)]
    pub currency: Option<String>,
    pub status: String,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub payout_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// User response from Tilled API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// List response wrapper — Tilled uses `items` as the array field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListResponse<T> {
    #[serde(alias = "data")]
    pub items: Vec<T>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub total: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedCurrency {
    Usd,
}

impl SupportedCurrency {
    pub fn as_str(self) -> &'static str {
        match self {
            SupportedCurrency::Usd => "usd",
        }
    }

    pub fn parse(input: &str) -> Result<Self, TilledError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "usd" => Ok(SupportedCurrency::Usd),
            other => Err(TilledError::ValidationError(format!(
                "Unsupported currency '{other}'. Supported: usd"
            ))),
        }
    }
}

pub fn normalize_currency(input: &str) -> Result<String, TilledError> {
    Ok(SupportedCurrency::parse(input)?.as_str().to_string())
}

pub fn checked_i32_to_i64(amount: i32) -> i64 {
    i64::from(amount)
}

pub fn checked_i64_to_i32(amount: i64) -> Result<i32, TilledError> {
    i32::try_from(amount).map_err(|_| {
        TilledError::ValidationError(format!("Amount {amount} is out of range for i32"))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        checked_i32_to_i64, checked_i64_to_i32, normalize_currency, BalanceTransaction,
        ConnectedAccount, Payout, SupportedCurrency, User,
    };

    #[test]
    fn currency_normalizes_to_lowercase_usd() {
        assert_eq!(normalize_currency("USD").unwrap(), "usd");
        assert_eq!(normalize_currency(" usd ").unwrap(), "usd");
        assert_eq!(
            SupportedCurrency::parse("usd").unwrap(),
            SupportedCurrency::Usd
        );
    }

    #[test]
    fn currency_rejects_unsupported_values() {
        let err = normalize_currency("eur").unwrap_err().to_string();
        assert!(err.contains("Unsupported currency"));
    }

    #[test]
    fn amount_conversion_round_trips_within_bounds() {
        let input = 12345_i32;
        let widened = checked_i32_to_i64(input);
        let narrowed = checked_i64_to_i32(widened).unwrap();
        assert_eq!(narrowed, input);
    }

    #[test]
    fn amount_conversion_rejects_out_of_range_i64() {
        assert!(checked_i64_to_i32(i64::from(i32::MAX) + 1).is_err());
    }

    #[test]
    fn connected_account_serialization_round_trip() {
        let value = serde_json::json!({
            "id": "acct_123",
            "status": "active",
            "email": "merchant@example.com"
        });
        let account: ConnectedAccount = serde_json::from_value(value).unwrap();
        assert_eq!(account.id, "acct_123");
        assert_eq!(account.email.as_deref(), Some("merchant@example.com"));
        assert!(account.capabilities.is_none());

        let serialized = serde_json::to_value(account).unwrap();
        assert_eq!(serialized.get("status").unwrap(), "active");
    }

    #[test]
    fn payout_serialization_round_trip() {
        let value = serde_json::json!({
            "id": "po_123",
            "status": "paid",
            "amount": 5000
        });
        let payout: Payout = serde_json::from_value(value).unwrap();
        assert_eq!(payout.id, "po_123");
        assert_eq!(payout.amount, Some(5000));
        assert!(payout.currency.is_none());

        let serialized = serde_json::to_value(payout).unwrap();
        assert_eq!(serialized.get("status").unwrap(), "paid");
    }

    #[test]
    fn balance_transaction_serialization_round_trip() {
        let value = serde_json::json!({
            "id": "bt_123",
            "amount": -22556.171,
            "status": "posted",
            "fee": -12.34
        });
        let txn: BalanceTransaction = serde_json::from_value(value).unwrap();
        assert_eq!(txn.id, "bt_123");
        assert_eq!(txn.amount, -22556.171);
        assert_eq!(txn.fee, Some(-12.34));
        assert!(txn.payout_id.is_none());

        let serialized = serde_json::to_value(txn).unwrap();
        assert_eq!(serialized.get("status").unwrap(), "posted");
    }

    #[test]
    fn user_serialization_round_trip() {
        let value = serde_json::json!({
            "id": "usr_123",
            "email": "merchant-user@example.com",
            "role": "merchant_admin",
            "status": "active"
        });
        let user: User = serde_json::from_value(value).unwrap();
        assert_eq!(user.id, "usr_123");
        assert_eq!(user.email.as_deref(), Some("merchant-user@example.com"));
        assert_eq!(user.role.as_deref(), Some("merchant_admin"));
        assert_eq!(user.status.as_deref(), Some("active"));
        assert!(user.created_at.is_none());
    }
}
