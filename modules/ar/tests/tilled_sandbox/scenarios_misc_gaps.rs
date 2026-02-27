//! Sandbox scenarios: remaining customer/charge/payment-method gaps (D4).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method, unique_email,
        RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::payment_method::{
        AddressRequest, BillingDetailsRequest, CardDetailsRequest, CreatePaymentMethodRequest,
        UpdatePaymentMethodRequest,
    };
    use std::collections::HashMap;

    fn sandbox_config() -> Option<(String, String, String)> {
        let sk = std::env::var("TILLED_SECRET_KEY").ok()?;
        let acct = std::env::var("TILLED_ACCOUNT_ID").ok()?;
        if sk.is_empty() || acct.is_empty() {
            eprintln!("SKIP: TILLED_SECRET_KEY / TILLED_ACCOUNT_ID not set");
            return None;
        }
        Some((sk, acct, "https://sandbox-api.tilled.com".to_string()))
    }

    #[tokio::test]
    async fn d4_list_customers() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let mut params = HashMap::new();
        params.insert("limit".to_string(), "50".to_string());
        let list = retry
            .execute(|| {
                let c = client.clone();
                let p = params.clone();
                async move { c.list_customers(Some(p)).await }
            })
            .await
            .expect("list_customers failed");

        eprintln!("[d4] list_customers returned {} items", list.items.len());
        assert!(!list.items.is_empty(), "expected sandbox to have customers");
        for c in &list.items {
            assert!(!c.id.is_empty());
        }
    }

    #[tokio::test]
    async fn d4_list_customers_with_filter_fallback() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let created = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                async move {
                    c.create_customer(e, Some("Filter Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        let mut filtered_params = HashMap::new();
        filtered_params.insert("email".to_string(), email.clone());
        filtered_params.insert("limit".to_string(), "25".to_string());

        let filtered = retry
            .execute(|| {
                let c = client.clone();
                let p = filtered_params.clone();
                async move { c.list_customers(Some(p)).await }
            })
            .await
            .expect("filtered list_customers failed");

        let found_filtered = filtered.items.iter().any(|c| c.id == created.id);
        if !found_filtered {
            let mut unfiltered_params = HashMap::new();
            unfiltered_params.insert("limit".to_string(), "100".to_string());
            let unfiltered = retry
                .execute(|| {
                    let c = client.clone();
                    let p = unfiltered_params.clone();
                    async move { c.list_customers(Some(p)).await }
                })
                .await
                .expect("fallback list_customers failed");
            assert!(
                unfiltered.items.iter().any(|c| c.id == created.id),
                "created customer should appear in list"
            );
        }

        cleanup_customer(&client, &created.id).await;
    }

    #[tokio::test]
    async fn d4_get_charge_by_id() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => return,
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let customer = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_customer(unique_email(), Some("Charge Get Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        let pm = try_create_test_payment_method(&sk, &acct, &base_url)
            .await
            .expect("create_test_payment_method failed");

        retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                let customer_id = customer.id.clone();
                async move { c.attach_payment_method(&pm_id, customer_id).await }
            })
            .await
            .expect("attach_payment_method failed");

        let charge_response = retry
            .execute(|| {
                let c = client.clone();
                let customer_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move {
                    c.create_charge(customer_id, pm_id, 1499, None, None, None)
                        .await
                }
            })
            .await
            .expect("create_charge failed");

        let charge_id = charge_response.charge_id.expect("expected charge_id");
        let charge = retry
            .execute(|| {
                let c = client.clone();
                let id = charge_id.clone();
                async move { c.get_charge(&id).await }
            })
            .await
            .expect("get_charge failed");

        assert_eq!(charge.id, charge_id);
        assert!(!charge.status.is_empty());
        if let Some(amount) = charge.amount {
            assert_eq!(amount, 1499);
        }

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    #[tokio::test]
    async fn d4_create_payment_method() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let req = CreatePaymentMethodRequest {
            payment_type: "card".to_string(),
            billing_details: Some(BillingDetailsRequest {
                name: Some("PM Create Test".to_string()),
                email: Some(unique_email()),
                address: Some(AddressRequest {
                    line1: None,
                    line2: None,
                    city: None,
                    state: None,
                    postal_code: None,
                    country: Some("US".to_string()),
                    zip: Some("90210".to_string()),
                }),
            }),
            card: Some(CardDetailsRequest {
                number: "4111111111111111".to_string(),
                exp_month: 12,
                exp_year: 2030,
                cvv: "123".to_string(),
            }),
            nick_name: Some("sandbox-create".to_string()),
        };

        let pm = retry
            .execute(|| {
                let c = client.clone();
                let r = req.clone();
                async move { c.create_payment_method(r).await }
            })
            .await
            .expect("create_payment_method failed");

        assert!(!pm.id.is_empty());
        assert_eq!(pm.payment_type, "card");

        cleanup_payment_method(&client, &pm.id).await;
    }

    #[tokio::test]
    async fn d4_update_payment_method() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| {
                let c = client.clone();
                let req = CreatePaymentMethodRequest {
                    payment_type: "card".to_string(),
                    billing_details: Some(BillingDetailsRequest {
                        name: Some("PM Update Before".to_string()),
                        email: Some(unique_email()),
                        address: None,
                    }),
                    card: Some(CardDetailsRequest {
                        number: "4111111111111111".to_string(),
                        exp_month: 12,
                        exp_year: 2030,
                        cvv: "123".to_string(),
                    }),
                    nick_name: Some("before".to_string()),
                };
                async move { c.create_payment_method(req).await }
            })
            .await
            .expect("create_payment_method failed");

        let updated = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                let req = UpdatePaymentMethodRequest {
                    billing_details: Some(BillingDetailsRequest {
                        name: Some("PM Update After".to_string()),
                        email: None,
                        address: None,
                    }),
                    nick_name: Some("after".to_string()),
                };
                async move { c.update_payment_method(&id, req).await }
            })
            .await
            .expect("update_payment_method failed");

        assert_eq!(updated.id, created.id);
        if let Some(details) = updated.billing_details {
            assert_eq!(details.name.as_deref(), Some("PM Update After"));
        }

        cleanup_payment_method(&client, &created.id).await;
    }
}
