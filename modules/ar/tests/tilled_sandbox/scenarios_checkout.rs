//! Sandbox scenarios: checkout session CRUD (create, get, list, expire).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::checkout_session::{
        CheckoutLineItem, CheckoutPriceData, CheckoutProductData, CreateCheckoutSessionRequest,
        PaymentIntentData,
    };
    use std::collections::HashMap;

    fn make_line_item(name: &str, amount: i64, qty: i64) -> CheckoutLineItem {
        CheckoutLineItem {
            quantity: qty,
            price_data: CheckoutPriceData {
                currency: "usd".to_string(),
                unit_amount: amount,
                product_data: CheckoutProductData {
                    name: name.to_string(),
                    description: None,
                },
            },
        }
    }

    fn make_request() -> CreateCheckoutSessionRequest {
        CreateCheckoutSessionRequest {
            line_items: vec![make_line_item("Test Product", 1500, 1)],
            payment_intent_data: PaymentIntentData {
                payment_method_types: vec!["card".to_string()],
            },
            success_url: None,
            cancel_url: None,
            customer_id: None,
            metadata: None,
        }
    }

    // -----------------------------------------------------------------------
    // Scenario CS1: Create a checkout session
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_cs1_create_checkout_session() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let session = retry
            .execute(|| client.create_checkout_session(make_request()))
            .await
            .expect("create_checkout_session should succeed");

        eprintln!(
            "[scenario-cs1] created checkout session: id={}, status={}, url={:?}",
            session.id, session.status, session.url
        );

        assert!(session.id.starts_with("cs_"), "id should start with cs_");
        assert_eq!(session.status, "open");
        assert!(
            session.url.is_some(),
            "url should be present for open session"
        );
        assert!(
            session.payment_intent_id.is_some(),
            "payment_intent_id should be set"
        );

        // Cleanup: expire the session
        let _ = client.expire_checkout_session(&session.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario CS2: Get a checkout session by ID
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_cs2_get_checkout_session() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let created = retry
            .execute(|| client.create_checkout_session(make_request()))
            .await
            .expect("create should succeed");

        let fetched = retry
            .execute(|| client.get_checkout_session(&created.id))
            .await
            .expect("get_checkout_session should succeed");

        eprintln!(
            "[scenario-cs2] fetched checkout session: id={}, status={}",
            fetched.id, fetched.status
        );

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.status, "open");
        assert_eq!(fetched.payment_intent_id, created.payment_intent_id);

        // Cleanup
        let _ = client.expire_checkout_session(&created.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario CS3: List checkout sessions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_cs3_list_checkout_sessions() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Create one to ensure at least one exists
        let session = retry
            .execute(|| client.create_checkout_session(make_request()))
            .await
            .expect("create should succeed");

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "5".to_string());

        let list = retry
            .execute(|| client.list_checkout_sessions(Some(filters.clone())))
            .await
            .expect("list_checkout_sessions should succeed");

        eprintln!(
            "[scenario-cs3] listed checkout sessions: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        assert!(!list.items.is_empty(), "should have at least one session");
        assert!(
            list.items.iter().any(|s| s.id == session.id),
            "created session should appear in list"
        );

        // Cleanup
        let _ = client.expire_checkout_session(&session.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario CS4: Expire a checkout session
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_cs4_expire_checkout_session() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let created = retry
            .execute(|| client.create_checkout_session(make_request()))
            .await
            .expect("create should succeed");

        assert_eq!(created.status, "open");

        let expired = retry
            .execute(|| client.expire_checkout_session(&created.id))
            .await
            .expect("expire_checkout_session should succeed");

        eprintln!(
            "[scenario-cs4] expired checkout session: id={}, status={}",
            expired.id, expired.status
        );

        assert_eq!(expired.id, created.id);
        assert_eq!(expired.status, "expired");
    }

    // -----------------------------------------------------------------------
    // Scenario CS5: Create checkout session with customer
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_cs5_create_with_customer() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Create a customer first
        use crate::tilled_sandbox::helpers::{cleanup_customer, unique_email};

        let email = unique_email();
        let customer = retry
            .execute(|| {
                client.create_customer(email.clone(), Some("Checkout Test".to_string()), None)
            })
            .await
            .expect("create_customer should succeed");

        let request = CreateCheckoutSessionRequest {
            line_items: vec![make_line_item("Customer Product", 2500, 2)],
            payment_intent_data: PaymentIntentData {
                payment_method_types: vec!["card".to_string()],
            },
            success_url: None,
            cancel_url: None,
            customer_id: Some(customer.id.clone()),
            metadata: None,
        };

        let session = retry
            .execute(|| client.create_checkout_session(request.clone()))
            .await
            .expect("create_checkout_session with customer should succeed");

        eprintln!(
            "[scenario-cs5] session with customer: id={}, customer_id={:?}, amount_total={:?}",
            session.id, session.customer_id, session.amount_total
        );

        assert!(session.id.starts_with("cs_"));
        assert_eq!(session.customer_id.as_deref(), Some(customer.id.as_str()));
        assert_eq!(session.amount_total, Some(5000)); // 2500 * 2

        // Cleanup
        let _ = client.expire_checkout_session(&session.id).await;
        cleanup_customer(&client, &customer.id).await;
    }
}
