//! Sandbox scenarios: platform fees.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    #[tokio::test]
    async fn scenario_pf1_list_platform_fees_merchant() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_platform_fees(None))
            .await
            .expect("list_platform_fees should succeed");

        // May be empty in sandbox — validate structure
        eprintln!(
            "[scenario-pf1] platform fees (merchant): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for fee in &list.items {
            assert!(!fee.id.is_empty(), "platform fee ID must be non-empty");
        }
    }

    #[tokio::test]
    async fn scenario_pf2_list_platform_fees_partner() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_platform_fees(None))
            .await
            .expect("list_platform_fees (partner) should succeed");

        eprintln!(
            "[scenario-pf2] platform fees (partner): total={:?}, items={}",
            list.total,
            list.items.len()
        );

        // If we have fees, verify structure
        if let Some(fee) = list.items.first() {
            assert!(!fee.id.is_empty());
            eprintln!(
                "[scenario-pf2] first fee: id={}, amount={:?}",
                fee.id, fee.amount
            );

            // Get by ID
            let fetched = retry
                .execute(|| client.get_platform_fee(&fee.id))
                .await
                .expect("get_platform_fee should succeed");
            assert_eq!(fetched.id, fee.id);
        }
    }
}
