//! Sandbox scenarios: API keys.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    #[tokio::test]
    async fn scenario_ak1_list_api_keys() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_api_keys(None))
            .await
            .expect("list_api_keys should succeed");

        eprintln!(
            "[scenario-ak1] api keys: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for key in &list.items {
            assert!(!key.id.is_empty(), "api key ID must be non-empty");
        }
    }

    #[tokio::test]
    async fn scenario_ak2_create_update_delete_publishable_key() {
        // API key CRUD may require partner scope
        let client = match try_partner_client() {
            Some(c) => c,
            None => match try_sandbox_client() {
                Some(c) => c,
                None => {
                    eprintln!("SKIP: sandbox creds not set");
                    return;
                }
            },
        };

        let retry = RetryPolicy::patient();
        let key_name = format!("test-key-{}", uuid::Uuid::new_v4());

        // Create publishable key — may fail with 4xx if scope doesn't allow it
        let created = match retry
            .execute(|| client.create_api_key("publishable".to_string(), Some(key_name.clone())))
            .await
        {
            Ok(k) => k,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if (400..500).contains(&status_code) => {
                eprintln!(
                    "[scenario-ak2] SKIP: cannot create API key at this scope ({}): {}",
                    status_code, message
                );
                return;
            }
            Err(e) => panic!("create_api_key failed unexpectedly: {e}"),
        };

        assert!(!created.id.is_empty());
        eprintln!(
            "[scenario-ak2] created key: id={}, type={:?}, name={:?}",
            created.id, created.key_type, created.name
        );

        // Update name
        let updated_name = format!("updated-{}", uuid::Uuid::new_v4());
        let updated = retry
            .execute(|| client.update_api_key(&created.id, Some(updated_name.clone())))
            .await
            .expect("update_api_key should succeed");
        assert_eq!(updated.id, created.id);
        eprintln!(
            "[scenario-ak2] updated key name: {:?} -> {:?}",
            created.name, updated.name
        );

        // Delete
        retry
            .execute(|| client.delete_api_key(&created.id))
            .await
            .expect("delete_api_key should succeed");
        eprintln!("[scenario-ak2] deleted key: {}", created.id);
    }

    #[tokio::test]
    async fn scenario_ak3_list_api_keys_partner_scope() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_api_keys(None))
            .await
            .expect("list_api_keys (partner) should succeed");

        eprintln!(
            "[scenario-ak3] api keys (partner): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for key in &list.items {
            assert!(!key.id.is_empty());
            eprintln!(
                "[scenario-ak3] key: id={}, type={:?}, scope={:?}",
                key.id, key.key_type, key.scope
            );
        }
    }
}
