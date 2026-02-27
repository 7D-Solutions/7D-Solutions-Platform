//! Sandbox scenarios: Apple Pay domains.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;

    #[tokio::test]
    async fn scenario_apd1_list_apple_pay_domains() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_apple_pay_domains())
            .await
            .expect("list_apple_pay_domains should succeed");

        eprintln!(
            "[scenario-apd1] apple pay domains: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for domain in &list.items {
            assert!(!domain.id.is_empty(), "domain ID must be non-empty");
        }
    }

    #[tokio::test]
    async fn scenario_apd2_create_get_delete_lifecycle() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let test_domain = format!("sandbox-test-{}.example.com", uuid::Uuid::new_v4());

        // Create — sandbox may reject test domains with 4xx
        let created = match retry
            .execute(|| client.create_apple_pay_domain(test_domain.clone()))
            .await
        {
            Ok(d) => d,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if (400..500).contains(&status_code) => {
                eprintln!(
                    "[scenario-apd2] SKIP: sandbox rejected test domain ({}): {}",
                    status_code, message
                );
                return;
            }
            Err(e) => panic!("create_apple_pay_domain failed unexpectedly: {e}"),
        };

        assert!(!created.id.is_empty());
        eprintln!(
            "[scenario-apd2] created domain: id={}, name={:?}",
            created.id, created.domain_name
        );

        // Get by ID
        let fetched = retry
            .execute(|| client.get_apple_pay_domain(&created.id))
            .await
            .expect("get_apple_pay_domain should succeed");
        assert_eq!(fetched.id, created.id);

        // Delete
        retry
            .execute(|| client.delete_apple_pay_domain(&created.id))
            .await
            .expect("delete_apple_pay_domain should succeed");
        eprintln!("[scenario-apd2] deleted domain: {}", created.id);
    }
}
