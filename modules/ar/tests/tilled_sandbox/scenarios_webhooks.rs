//! Sandbox scenarios: webhook endpoints CRUD (create, list, get, update, delete).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;

    /// Best-effort cleanup of a webhook endpoint.
    async fn cleanup_webhook_endpoint(client: &ar_rs::tilled::TilledClient, endpoint_id: &str) {
        match client.delete_webhook_endpoint(endpoint_id).await {
            Ok(_) => eprintln!("[sandbox-cleanup] deleted webhook endpoint {endpoint_id}"),
            Err(e) => {
                eprintln!("[sandbox-cleanup] could not delete webhook endpoint {endpoint_id}: {e}")
            }
        }
    }

    #[tokio::test]
    async fn scenario_wh1_create_webhook_endpoint() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let url = "https://webhook-test.7d-solutions.example.com/tilled/wh1".to_string();
        let events = vec!["payment_intent.succeeded".to_string()];

        let created = retry
            .execute(|| {
                let c = client.clone();
                let u = url.clone();
                let e = events.clone();
                async move {
                    c.create_webhook_endpoint(u, e, Some("wh1 test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_webhook_endpoint failed");

        eprintln!(
            "[scenario-wh1] created webhook endpoint: {} url={}",
            created.id, created.url
        );
        assert!(!created.id.is_empty());
        assert_eq!(created.url, url);

        // Cleanup
        cleanup_webhook_endpoint(&client, &created.id).await;
    }

    #[tokio::test]
    async fn scenario_wh2_list_webhook_endpoints() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        // Create one to ensure list is non-empty
        let created = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_webhook_endpoint(
                        "https://webhook-test.7d-solutions.example.com/tilled/wh2".to_string(),
                        vec!["*".to_string()],
                        Some("wh2 list test".to_string()),
                        None,
                    )
                    .await
                }
            })
            .await
            .expect("create_webhook_endpoint failed");

        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_webhook_endpoints().await }
            })
            .await
            .expect("list_webhook_endpoints failed");

        eprintln!(
            "[scenario-wh2] listed {} webhook endpoints",
            list.items.len()
        );
        assert!(
            !list.items.is_empty(),
            "should have at least one webhook endpoint"
        );

        // Cleanup
        cleanup_webhook_endpoint(&client, &created.id).await;
    }

    #[tokio::test]
    async fn scenario_wh3_get_webhook_endpoint() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_webhook_endpoint(
                        "https://webhook-test.7d-solutions.example.com/tilled/wh3".to_string(),
                        vec!["payment_intent.created".to_string()],
                        Some("wh3 get test".to_string()),
                        None,
                    )
                    .await
                }
            })
            .await
            .expect("create_webhook_endpoint failed");

        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.get_webhook_endpoint(&id).await }
            })
            .await
            .expect("get_webhook_endpoint failed");

        eprintln!("[scenario-wh3] fetched webhook endpoint: {}", fetched.id);
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.url, created.url);

        // Cleanup
        cleanup_webhook_endpoint(&client, &created.id).await;
    }

    #[tokio::test]
    async fn scenario_wh4_update_webhook_endpoint() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_webhook_endpoint(
                        "https://webhook-test.7d-solutions.example.com/tilled/wh4".to_string(),
                        vec!["payment_intent.succeeded".to_string()],
                        Some("wh4 before update".to_string()),
                        None,
                    )
                    .await
                }
            })
            .await
            .expect("create_webhook_endpoint failed");

        let new_url =
            "https://webhook-test.7d-solutions.example.com/tilled/wh4-updated".to_string();
        let updated = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                let u = new_url.clone();
                async move {
                    c.update_webhook_endpoint(
                        &id,
                        Some(u),
                        None,
                        Some("wh4 after update".to_string()),
                    )
                    .await
                }
            })
            .await
            .expect("update_webhook_endpoint failed");

        eprintln!(
            "[scenario-wh4] updated webhook endpoint: {} url={}",
            updated.id, updated.url
        );
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.url, new_url);

        // Cleanup
        cleanup_webhook_endpoint(&client, &created.id).await;
    }

    #[tokio::test]
    async fn scenario_wh5_delete_webhook_endpoint() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let created = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_webhook_endpoint(
                        "https://webhook-test.7d-solutions.example.com/tilled/wh5".to_string(),
                        vec!["*".to_string()],
                        Some("wh5 delete test".to_string()),
                        None,
                    )
                    .await
                }
            })
            .await
            .expect("create_webhook_endpoint failed");

        retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.delete_webhook_endpoint(&id).await }
            })
            .await
            .expect("delete_webhook_endpoint failed");

        eprintln!("[scenario-wh5] deleted webhook endpoint: {}", created.id);

        // Verify it's gone from list
        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_webhook_endpoints().await }
            })
            .await
            .expect("list_webhook_endpoints failed");

        let still_present = list.items.iter().any(|we| we.id == created.id);
        assert!(
            !still_present,
            "deleted webhook endpoint should no longer appear in list"
        );
    }

    #[tokio::test]
    async fn scenario_wh6_full_lifecycle() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        // 1. Create
        let created = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.create_webhook_endpoint(
                        "https://webhook-test.7d-solutions.example.com/tilled/wh6".to_string(),
                        vec![
                            "payment_intent.succeeded".to_string(),
                            "payment_intent.payment_failed".to_string(),
                        ],
                        Some("wh6 lifecycle test".to_string()),
                        None,
                    )
                    .await
                }
            })
            .await
            .expect("create failed");

        eprintln!("[scenario-wh6] created: {}", created.id);
        assert!(!created.id.is_empty());

        // 2. Get
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.get_webhook_endpoint(&id).await }
            })
            .await
            .expect("get failed");
        assert_eq!(fetched.id, created.id);

        // 3. Update
        let updated = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move {
                    c.update_webhook_endpoint(
                        &id,
                        Some(
                            "https://webhook-test.7d-solutions.example.com/tilled/wh6-v2"
                                .to_string(),
                        ),
                        Some(vec!["*".to_string()]),
                        Some("wh6 updated".to_string()),
                    )
                    .await
                }
            })
            .await
            .expect("update failed");
        assert_eq!(
            updated.url,
            "https://webhook-test.7d-solutions.example.com/tilled/wh6-v2"
        );
        eprintln!("[scenario-wh6] updated url: {}", updated.url);

        // 4. Delete
        retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.delete_webhook_endpoint(&id).await }
            })
            .await
            .expect("delete failed");
        eprintln!("[scenario-wh6] deleted: {}", created.id);

        // Verify gone
        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_webhook_endpoints().await }
            })
            .await
            .expect("list failed");
        assert!(
            !list.items.iter().any(|we| we.id == created.id),
            "endpoint should be gone after delete"
        );
    }
}
