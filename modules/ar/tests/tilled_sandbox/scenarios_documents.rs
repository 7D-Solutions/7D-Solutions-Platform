//! Sandbox scenarios: documents.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    #[tokio::test]
    async fn scenario_doc1_list_documents_merchant() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_documents(None))
            .await
            .expect("list_documents should succeed");

        // May be empty — validate response structure
        eprintln!(
            "[scenario-doc1] documents (merchant): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for doc in &list.items {
            assert!(!doc.id.is_empty(), "document ID must be non-empty");
        }
    }

    #[tokio::test]
    async fn scenario_doc2_list_documents_partner() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_documents(None))
            .await
            .expect("list_documents (partner) should succeed");

        eprintln!(
            "[scenario-doc2] documents (partner): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        // If we have documents, verify get-by-ID
        if let Some(doc) = list.items.first() {
            assert!(!doc.id.is_empty());
            let fetched = retry
                .execute(|| client.get_document(&doc.id))
                .await
                .expect("get_document should succeed");
            assert_eq!(fetched.id, doc.id);
            eprintln!(
                "[scenario-doc2] fetched document: id={}, status={:?}",
                fetched.id, fetched.status
            );
        }
    }
}
