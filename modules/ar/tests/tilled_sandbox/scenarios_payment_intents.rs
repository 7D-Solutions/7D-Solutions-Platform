//! Sandbox scenarios: payment intent list, get, update, cancel.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::payment_intent::{CreatePaymentIntentRequest, UpdatePaymentIntentRequest};
    use std::collections::HashMap;

    fn make_pi_request(amount: i64) -> CreatePaymentIntentRequest {
        CreatePaymentIntentRequest {
            amount,
            currency: "usd".to_string(),
            payment_method_types: vec!["card".to_string()],
            customer_id: None,
            payment_method_id: None,
            description: Some("sandbox-pi-test".to_string()),
            metadata: None,
            confirm: None,
            capture_method: None,
        }
    }

    // -----------------------------------------------------------------------
    // Scenario PI1: List payment intents with limit
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_pi1_list_payment_intents() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Create one to ensure at least one exists
        let created = retry
            .execute(|| client.create_payment_intent(make_pi_request(1100)))
            .await
            .expect("create_payment_intent should succeed");

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "5".to_string());

        let list = retry
            .execute(|| client.list_payment_intents(Some(filters.clone())))
            .await
            .expect("list_payment_intents should succeed");

        eprintln!(
            "[scenario-pi1] listed payment intents: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        assert!(!list.items.is_empty(), "should have at least one PI");
        for item in &list.items {
            assert!(item.id.starts_with("pi_"), "id should start with pi_");
            assert!(item.amount > 0, "amount should be positive");
            assert!(!item.status.is_empty(), "status should not be empty");
        }

        // Cleanup: cancel the created PI
        let _ = client.cancel_payment_intent(&created.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario PI2: Get payment intent by ID
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_pi2_get_payment_intent() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| client.create_payment_intent(make_pi_request(2200)))
            .await
            .expect("create_payment_intent should succeed");

        let fetched = retry
            .execute(|| client.get_payment_intent(&created.id))
            .await
            .expect("get_payment_intent should succeed");

        eprintln!(
            "[scenario-pi2] fetched PI: id={}, status={}, amount={}",
            fetched.id, fetched.status, fetched.amount
        );

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.amount, 2200);
        assert_eq!(fetched.currency, "usd");
        assert_eq!(fetched.status, created.status);

        // Cleanup
        let _ = client.cancel_payment_intent(&created.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario PI3: Update payment intent (amount + metadata)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_pi3_update_payment_intent() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| client.create_payment_intent(make_pi_request(3300)))
            .await
            .expect("create_payment_intent should succeed");

        assert_eq!(created.amount, 3300);

        let mut meta = HashMap::new();
        meta.insert("updated_by".to_string(), "sandbox-test".to_string());

        let updated = retry
            .execute(|| {
                client.update_payment_intent(&created.id, UpdatePaymentIntentRequest {
                    amount: Some(4400),
                    currency: None,
                    metadata: Some(meta.clone()),
                })
            })
            .await
            .expect("update_payment_intent should succeed");

        eprintln!(
            "[scenario-pi3] updated PI: id={}, amount={}, metadata={:?}",
            updated.id, updated.amount, updated.metadata
        );

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.amount, 4400);
        assert_eq!(
            updated.metadata.as_ref().and_then(|m| m.get("updated_by")).map(|s| s.as_str()),
            Some("sandbox-test")
        );

        // Verify via GET
        let verified = retry
            .execute(|| client.get_payment_intent(&created.id))
            .await
            .expect("get after update should succeed");
        assert_eq!(verified.amount, 4400);

        // Cleanup
        let _ = client.cancel_payment_intent(&created.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario PI4: Cancel payment intent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_pi4_cancel_payment_intent() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| client.create_payment_intent(make_pi_request(5500)))
            .await
            .expect("create_payment_intent should succeed");

        assert_eq!(created.status, "requires_payment_method");

        let canceled = retry
            .execute(|| client.cancel_payment_intent(&created.id))
            .await
            .expect("cancel_payment_intent should succeed");

        eprintln!(
            "[scenario-pi4] canceled PI: id={}, status={}",
            canceled.id, canceled.status
        );

        assert_eq!(canceled.id, created.id);
        assert_eq!(canceled.status, "canceled");

        // Verify via GET
        let verified = retry
            .execute(|| client.get_payment_intent(&created.id))
            .await
            .expect("get after cancel should succeed");
        assert_eq!(verified.status, "canceled");
    }

    // -----------------------------------------------------------------------
    // Scenario PI5: List with status filter
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_pi5_list_with_status_filter() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        let mut filters = HashMap::new();
        filters.insert("status".to_string(), "succeeded".to_string());
        filters.insert("limit".to_string(), "5".to_string());

        let list = retry
            .execute(|| client.list_payment_intents(Some(filters.clone())))
            .await
            .expect("list_payment_intents with status filter should succeed");

        eprintln!(
            "[scenario-pi5] filtered list (status=succeeded): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for item in &list.items {
            assert_eq!(
                item.status, "succeeded",
                "all items should have status=succeeded, got {}",
                item.status
            );
        }
    }
}
