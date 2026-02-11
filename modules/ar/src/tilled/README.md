# Tilled API Client for Rust

A Rust client library for the Tilled payment processor API.

## Features

- **Customer Management**: Create, read, update, and delete customers
- **Payment Methods**: Attach, detach, and list payment methods
- **Payment Intents**: Create charges and payment intents
- **Subscriptions**: Manage recurring subscriptions
- **Refunds**: Process and track refunds
- **Disputes**: Retrieve and list disputes
- **Webhook Verification**: Secure webhook signature verification

## Configuration

The client is configured using environment variables per app:

```bash
TILLED_SECRET_KEY_{APP_ID}=tsk_...
TILLED_ACCOUNT_ID_{APP_ID}=acct_...
TILLED_WEBHOOK_SECRET_{APP_ID}=whsec_...
TILLED_SANDBOX=true  # or false for production
```

## Usage

### Initialize the Client

```rust
use ar_rs::tilled::{TilledClient, TilledConfig};

// From environment variables
let client = TilledClient::from_env("myapp")?;

// Or with explicit configuration
let config = TilledConfig {
    secret_key: "tsk_...".to_string(),
    account_id: "acct_...".to_string(),
    webhook_secret: "whsec_...".to_string(),
    sandbox: true,
    base_path: "https://sandbox-api.tilled.com".to_string(),
};
let client = TilledClient::new(config)?;
```

### Customer Operations

```rust
// Create a customer
let customer = client.create_customer(
    "customer@example.com".to_string(),
    Some("John Doe".to_string()),
    None,
).await?;

// Get a customer
let customer = client.get_customer("cus_123").await?;

// Update a customer
use ar_rs::tilled::customer::UpdateCustomerRequest;
let updated = client.update_customer(
    "cus_123",
    UpdateCustomerRequest {
        email: Some("newemail@example.com".to_string()),
        first_name: Some("Jane".to_string()),
        last_name: None,
        metadata: None,
    },
).await?;

// Delete a customer
client.delete_customer("cus_123").await?;
```

### Payment Methods

```rust
// Attach a payment method to a customer
let payment_method = client.attach_payment_method(
    "pm_123",
    "cus_123".to_string(),
).await?;

// List customer's payment methods
let methods = client.list_payment_methods("cus_123").await?;

// Detach a payment method
client.detach_payment_method("pm_123").await?;
```

### Charges (One-time Payments)

```rust
// Create a charge
let charge = client.create_charge(
    "cus_123".to_string(),
    "pm_123".to_string(),
    1000, // Amount in cents ($10.00)
    Some("usd".to_string()),
    Some("Product purchase".to_string()),
    None,
).await?;

println!("Charge status: {}", charge.status);
```

### Subscriptions

```rust
use ar_rs::tilled::subscription::SubscriptionOptions;

// Create a subscription
let subscription = client.create_subscription(
    "cus_123".to_string(),
    "pm_123".to_string(),
    999, // $9.99/month
    Some(SubscriptionOptions {
        interval_unit: Some("month".to_string()),
        interval_count: Some(1),
        trial_end: Some(1735689600), // Unix timestamp
        ..Default::default()
    }),
).await?;

// Update a subscription
use ar_rs::tilled::subscription::UpdateSubscriptionRequest;
client.update_subscription(
    "sub_123",
    UpdateSubscriptionRequest {
        payment_method_id: Some("pm_456".to_string()),
        cancel_at_period_end: Some(true),
        ..Default::default()
    },
).await?;

// Cancel a subscription
client.cancel_subscription("sub_123").await?;
```

### Refunds

```rust
// Create a refund
let refund = client.create_refund(
    "pi_123".to_string(), // Payment intent ID
    500, // Refund amount in cents
    Some("usd".to_string()),
    Some("Customer request".to_string()),
    None,
).await?;

// Get a refund
let refund = client.get_refund("re_123").await?;

// List refunds
let refunds = client.list_refunds(None).await?;
```

### Webhook Verification

```rust
use ar_rs::tilled::webhook::verify_webhook_signature;

// In your webhook handler
fn handle_webhook(raw_body: String, signature: String) -> Result<(), TilledError> {
    let webhook_secret = "whsec_...";

    // Verify the signature
    verify_webhook_signature(&raw_body, &signature, webhook_secret, Some(300))?;

    // Parse and process the webhook
    let event: serde_json::Value = serde_json::from_str(&raw_body)?;

    match event["type"].as_str() {
        Some("payment_intent.succeeded") => {
            // Handle successful payment
        },
        Some("subscription.created") => {
            // Handle new subscription
        },
        _ => {}
    }

    Ok(())
}

// Or use the config method
let config = TilledConfig::from_env("myapp")?;
config.verify_webhook(&raw_body, &signature, Some(300))?;
```

## Error Handling

The client returns `Result<T, TilledError>` for all operations:

```rust
use ar_rs::tilled::error::TilledError;

match client.create_customer(email, name, None).await {
    Ok(customer) => println!("Customer created: {}", customer.id),
    Err(TilledError::ApiError { status_code, message }) => {
        eprintln!("API error {}: {}", status_code, message);
    },
    Err(TilledError::ConfigError(msg)) => {
        eprintln!("Configuration error: {}", msg);
    },
    Err(e) => {
        eprintln!("Error: {}", e);
    },
}
```

## API Documentation

For detailed Tilled API documentation, see: https://docs.tilled.com/api

## Testing

Run the webhook verification tests:

```bash
cargo test --lib tilled::webhook
```

## Security Notes

- Always verify webhook signatures to ensure requests are from Tilled
- Use the tolerance parameter (default: 300 seconds) to prevent replay attacks
- Store API keys securely in environment variables or secrets management
- Use sandbox mode for development and testing
