//! Sandbox scenarios: events and balance-transaction point lookups.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;
    use std::collections::HashMap;

    #[tokio::test]
    async fn scenario_e1_list_events() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "10".to_string());

        let retry = RetryPolicy::default();
        let response = retry
            .execute(|| client.list_events(Some(filters.clone())))
            .await
            .expect("list_events should succeed");

        eprintln!(
            "[scenario-e1] events: total={:?}, items_returned={}",
            response.total,
            response.items.len()
        );
        assert!(
            !response.items.is_empty(),
            "sandbox should have at least one event"
        );

        for event in &response.items {
            assert!(!event.id.is_empty(), "event ID must be non-empty");
            assert!(
                !event.event_type.is_empty(),
                "event type must be non-empty for {}",
                event.id
            );
        }
    }

    #[tokio::test]
    async fn scenario_e2_get_event() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_events(None))
            .await
            .expect("list_events should succeed");
        let first = list.items.first().expect("expected at least one event");

        let fetched = retry
            .execute(|| client.get_event(&first.id))
            .await
            .expect("get_event should succeed");

        assert_eq!(fetched.id, first.id);
        assert_eq!(fetched.event_type, first.event_type);
        assert!(
            !fetched.event_type.is_empty(),
            "event type must be non-empty"
        );
    }

    #[tokio::test]
    async fn scenario_e3_get_balance_transaction() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_balance_transactions(None, Some(20)))
            .await
            .expect("list_balance_transactions should succeed");
        let first = list
            .items
            .first()
            .expect("expected at least one balance transaction");

        let fetched = retry
            .execute(|| client.get_balance_transaction(&first.id))
            .await
            .expect("get_balance_transaction should succeed");

        assert_eq!(fetched.id, first.id);
        assert!(!fetched.status.is_empty(), "status must be non-empty");
    }

    #[tokio::test]
    async fn scenario_e4_get_balance_transaction_summary() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "10".to_string());

        let retry = RetryPolicy::default();
        let summary = retry
            .execute(|| client.get_balance_transaction_summary(Some(filters.clone())))
            .await
            .expect("get_balance_transaction_summary should succeed");

        assert!(
            summary.data.is_object(),
            "summary response should deserialize into an object"
        );
        let keys = summary
            .data
            .as_object()
            .expect("summary should be object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        eprintln!("[scenario-e4] summary keys: {:?}", keys);
        assert!(!keys.is_empty(), "summary payload should contain fields");
    }
}
