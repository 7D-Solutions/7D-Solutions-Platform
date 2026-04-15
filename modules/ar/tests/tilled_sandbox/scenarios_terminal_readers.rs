//! Sandbox scenarios: terminal readers.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_sandbox_client;

    #[tokio::test]
    async fn scenario_tr1_list_terminal_readers() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_terminal_readers(None))
            .await
            .expect("list_terminal_readers should succeed");

        eprintln!(
            "[scenario-tr1] terminal readers: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        // Sandbox may have no readers — validate structure
        for reader in &list.items {
            assert!(
                !reader.id.is_empty(),
                "terminal reader ID must be non-empty"
            );
        }
    }

    #[tokio::test]
    async fn scenario_tr2_get_and_status_if_readers_exist() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_terminal_readers(None))
            .await
            .expect("list_terminal_readers should succeed");

        let reader = match list.items.first() {
            Some(r) => r,
            None => {
                eprintln!("[scenario-tr2] SKIP: no terminal readers in sandbox");
                return;
            }
        };

        // Get by ID
        let fetched = retry
            .execute(|| client.get_terminal_reader(&reader.id))
            .await
            .expect("get_terminal_reader should succeed");
        assert_eq!(fetched.id, reader.id);
        eprintln!(
            "[scenario-tr2] reader: id={}, label={:?}, status={:?}",
            fetched.id, fetched.label, fetched.status
        );

        // Get connection status
        let status = retry
            .execute(|| client.get_terminal_reader_status(&reader.id))
            .await
            .expect("get_terminal_reader_status should succeed");
        eprintln!(
            "[scenario-tr2] status: connected={:?}, status={:?}",
            status.connected, status.status
        );
    }

    #[tokio::test]
    async fn scenario_tr3_update_label_if_readers_exist() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_terminal_readers(None))
            .await
            .expect("list_terminal_readers should succeed");

        let reader = match list.items.first() {
            Some(r) => r,
            None => {
                eprintln!("[scenario-tr3] SKIP: no terminal readers in sandbox");
                return;
            }
        };

        let original_label = reader.label.clone();
        let test_label = format!("Test Label {}", uuid::Uuid::new_v4());

        // Update label
        let updated = retry
            .execute(|| client.update_terminal_reader(&reader.id, Some(test_label.clone())))
            .await
            .expect("update_terminal_reader should succeed");
        assert_eq!(updated.id, reader.id);
        eprintln!(
            "[scenario-tr3] updated label: {:?} -> {:?}",
            original_label, updated.label
        );

        // Restore original label
        if let Some(orig) = original_label {
            let _ = retry
                .execute(|| client.update_terminal_reader(&reader.id, Some(orig.clone())))
                .await;
        }
    }
}
