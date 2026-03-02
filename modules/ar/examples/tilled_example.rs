/// Example usage of the Tilled API client
///
/// This example demonstrates how to use the Tilled client for common operations.
/// To run this example:
///
/// 1. Set up environment variables:
///    ```bash
///    export TILLED_SECRET_KEY_MYAPP=tsk_...
///    export TILLED_ACCOUNT_ID_MYAPP=acct_...
///    export TILLED_WEBHOOK_SECRET_MYAPP=whsec_...
///    export TILLED_SANDBOX=true
///    ```
///
/// 2. Run the example:
///    ```bash
///    cargo run --example tilled_example
///    ```
use ar_rs::tilled::{
    customer::UpdateCustomerRequest, subscription::SubscriptionOptions, TilledClient, TilledConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize from environment variables
    let client = TilledClient::from_env("myapp")?;

    println!("🔧 Tilled API Client Example\n");

    // Example 1: Create a customer
    println!("1️⃣  Creating a customer...");
    let customer = client
        .create_customer(
            "customer@example.com".to_string(),
            Some("John Doe".to_string()),
            None,
        )
        .await?;
    println!("   ✅ Customer created: {}", customer.id);

    // Example 2: Update the customer
    println!("\n2️⃣  Updating customer...");
    let updated_customer = client
        .update_customer(
            &customer.id,
            UpdateCustomerRequest {
                email: Some("newemail@example.com".to_string()),
                first_name: Some("Jane".to_string()),
                last_name: Some("Smith".to_string()),
                metadata: None,
            },
        )
        .await?;
    println!("   ✅ Customer updated: {:?}", updated_customer.email);

    // Example 3: Create a subscription (requires a payment method)
    // Note: In a real scenario, you'd first create a payment method
    println!("\n3️⃣  Creating a subscription (example - requires payment method)");
    println!("   ℹ️  Skipped - requires a valid payment method ID");
    /*
    let subscription = client
        .create_subscription(
            customer.id.clone(),
            "pm_test_123".to_string(),
            999, // $9.99/month
            Some(SubscriptionOptions {
                interval_unit: Some("month".to_string()),
                interval_count: Some(1),
                ..Default::default()
            }),
        )
        .await?;
    println!("   ✅ Subscription created: {}", subscription.id);
    */

    // Example 4: Webhook verification
    println!("\n4️⃣  Webhook signature verification");
    use ar_rs::tilled::webhook::verify_webhook_signature;

    let test_body = r#"{"type":"payment_intent.succeeded","data":{"id":"pi_123"}}"#;
    let test_secret = "whsec_test_secret";

    // Generate a test signature
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::time::{SystemTime, UNIX_EPOCH};

    type HmacSha256 = Hmac<Sha256>;

    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let signed_payload = format!("{}.{}", timestamp, test_body);
    let mut mac = HmacSha256::new_from_slice(test_secret.as_bytes())?;
    mac.update(signed_payload.as_bytes());
    let signature_hash = hex::encode(mac.finalize().into_bytes());
    let signature = format!("t={},v1={}", timestamp, signature_hash);

    match verify_webhook_signature(test_body, &signature, test_secret, Some(300)) {
        Ok(_) => println!("   ✅ Webhook signature verified successfully"),
        Err(e) => println!("   ❌ Webhook verification failed: {}", e),
    }

    // Example 5: Delete the customer
    println!("\n5️⃣  Deleting customer...");
    client.delete_customer(&customer.id).await?;
    println!("   ✅ Customer deleted");

    println!("\n✨ All examples completed successfully!");

    Ok(())
}
