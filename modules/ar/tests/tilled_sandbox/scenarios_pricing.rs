//! Sandbox scenarios: pricing templates (partner scope).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_partner_client;

    #[tokio::test]
    async fn scenario_pt1_list_pricing_templates() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_pricing_templates(None))
            .await
            .expect("list_pricing_templates should succeed");

        assert!(
            !list.items.is_empty(),
            "partner account should have pricing templates"
        );
        eprintln!(
            "[scenario-pt1] pricing templates: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for pt in &list.items {
            assert!(!pt.id.is_empty(), "pricing template ID must be non-empty");
            eprintln!(
                "  id={}, name={:?}, type={:?}, currency={:?}",
                pt.id, pt.name, pt.payment_method_type, pt.currency
            );
        }
    }

    #[tokio::test]
    async fn scenario_pt2_get_pricing_template() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_pricing_templates(None))
            .await
            .expect("list_pricing_templates should succeed");

        let first = list
            .items
            .first()
            .expect("expected at least one pricing template");

        let fetched = retry
            .execute(|| client.get_pricing_template(&first.id))
            .await
            .expect("get_pricing_template should succeed");

        assert_eq!(fetched.id, first.id);
        assert!(
            fetched.status.is_some(),
            "pricing template should have a status"
        );
        eprintln!(
            "[scenario-pt2] fetched template: id={}, status={:?}",
            fetched.id, fetched.status
        );
    }
}
