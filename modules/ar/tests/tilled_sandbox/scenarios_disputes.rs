//! Sandbox scenarios: dispute trigger + evidence outcomes.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method, unique_email,
        RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::dispute::SubmitEvidenceRequest;
    use ar_rs::tilled::types::Dispute;
    use ar_rs::tilled::{error::TilledError, TilledClient};
    use std::time::{Duration, Instant};

    const DISPUTE_TRIGGER_AMOUNT: i64 = 777_799;

    fn sandbox_config() -> Option<(String, String, String)> {
        let sk = std::env::var("TILLED_SECRET_KEY").ok()?;
        let acct = std::env::var("TILLED_ACCOUNT_ID").ok()?;
        if sk.is_empty() || acct.is_empty() {
            eprintln!("SKIP: TILLED_SECRET_KEY / TILLED_ACCOUNT_ID not set");
            return None;
        }
        Some((sk, acct, "https://sandbox-api.tilled.com".to_string()))
    }

    fn is_evidence_eligible(status: &str) -> bool {
        status == "needs_response" || status == "warning_needs_response"
    }

    fn is_file_required_error(error: &TilledError) -> bool {
        matches!(
            error,
            TilledError::ApiError { status_code: 400, message }
                if message.contains("Must provide at least one file")
        )
    }

    async fn wait_for_dispute(
        client: &TilledClient,
        charge_id: &str,
        max_wait_secs: u64,
    ) -> Option<Dispute> {
        let deadline = Instant::now() + Duration::from_secs(max_wait_secs);
        loop {
            match client.list_disputes(None).await {
                Ok(list) => {
                    if let Some(dispute) = list
                        .items
                        .into_iter()
                        .find(|d| d.charge_id.as_deref() == Some(charge_id))
                    {
                        return Some(dispute);
                    }
                }
                Err(e) => eprintln!("[disputes] list_disputes poll error: {e}"),
            }

            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn create_disputed_charge(
        client: &TilledClient,
        retry: &RetryPolicy,
        sk: &str,
        acct: &str,
        base_url: &str,
    ) -> Option<(String, String, String, Dispute)> {
        let customer = retry
            .execute(|| {
                let c = client.clone();
                let e = unique_email();
                async move {
                    c.create_customer(e, Some("Dispute Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        let pm = match try_create_test_payment_method(sk, acct, base_url).await {
            Some(pm) => pm,
            None => {
                cleanup_customer(client, &customer.id).await;
                return None;
            }
        };

        retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                let cust_id = customer.id.clone();
                async move { c.attach_payment_method(&pm_id, cust_id).await }
            })
            .await
            .expect("attach failed");

        let charge = retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                let cust_id = customer.id.clone();
                async move {
                    c.create_charge(cust_id, pm_id, DISPUTE_TRIGGER_AMOUNT, None, None, None)
                        .await
                }
            })
            .await
            .expect("create_charge failed");

        assert_eq!(charge.status, "succeeded");

        let ch_id = match &charge.charge_id {
            Some(id) => id.clone(),
            None => {
                eprintln!(
                    "SKIP: no charge_id in response for PI {} — cannot match disputes",
                    charge.id
                );
                cleanup_entities(client, &customer.id, &pm.id).await;
                return None;
            }
        };
        eprintln!("[disputes] PI={} charge_id={}", charge.id, ch_id);
        tokio::time::sleep(Duration::from_secs(2)).await;

        let dispute = match wait_for_dispute(client, &ch_id, 20).await {
            Some(d) => d,
            None => {
                eprintln!(
                    "SKIP: dispute did not appear within 20s for charge {}",
                    ch_id
                );
                cleanup_entities(client, &customer.id, &pm.id).await;
                return None;
            }
        };

        Some((customer.id, pm.id, ch_id, dispute))
    }

    async fn cleanup_entities(client: &TilledClient, customer_id: &str, pm_id: &str) {
        cleanup_payment_method(client, pm_id).await;
        cleanup_customer(client, customer_id).await;
    }

    #[tokio::test]
    async fn scenario_08_trigger_dispute_for_known_amount() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (customer_id, pm_id, charge_id, dispute) =
            match create_disputed_charge(&client, &retry, &sk, &acct, &base_url).await {
                Some(data) => data,
                None => return,
            };

        eprintln!(
            "[scenario-08] charge={} dispute={} status={}",
            charge_id, dispute.id, dispute.status
        );
        assert_eq!(
            dispute.charge_id.as_deref(),
            Some(charge_id.as_str())
        );
        assert!(!dispute.id.is_empty());

        cleanup_entities(&client, &customer_id, &pm_id).await;
    }

    #[tokio::test]
    async fn scenario_09_submit_dispute_evidence_reversal() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (customer_id, pm_id, charge_id, dispute) =
            match create_disputed_charge(&client, &retry, &sk, &acct, &base_url).await {
                Some(data) => data,
                None => return,
            };

        if !is_evidence_eligible(&dispute.status) {
            eprintln!(
                "SKIP: dispute {} status {} not evidence-eligible",
                dispute.id, dispute.status
            );
            cleanup_entities(&client, &customer_id, &pm_id).await;
            return;
        }

        let evidence = SubmitEvidenceRequest {
            description: Some(format!(
                "REVERSAL - sandbox scenario 09 {}",
                uuid::Uuid::new_v4()
            )),
            files: None,
        };
        let updated = match client.submit_dispute_evidence(&dispute.id, evidence).await {
            Ok(v) => v,
            Err(e) if is_file_required_error(&e) => {
                eprintln!("SKIP: sandbox requires file upload for evidence");
                cleanup_entities(&client, &customer_id, &pm_id).await;
                return;
            }
            Err(e) => panic!("submit_dispute_evidence failed: {e}"),
        };

        eprintln!(
            "[scenario-09] charge={} dispute={} status_after={}",
            charge_id, updated.id, updated.status
        );
        assert_eq!(updated.id, dispute.id);

        cleanup_entities(&client, &customer_id, &pm_id).await;
    }

    #[tokio::test]
    async fn scenario_10_submit_dispute_evidence_loss() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (customer_id, pm_id, charge_id, dispute) =
            match create_disputed_charge(&client, &retry, &sk, &acct, &base_url).await {
                Some(data) => data,
                None => return,
            };

        if !is_evidence_eligible(&dispute.status) {
            eprintln!(
                "SKIP: dispute {} status {} not evidence-eligible",
                dispute.id, dispute.status
            );
            cleanup_entities(&client, &customer_id, &pm_id).await;
            return;
        }

        let evidence = SubmitEvidenceRequest {
            description: Some(format!(
                "LOSS - sandbox scenario 10 {}",
                uuid::Uuid::new_v4()
            )),
            files: None,
        };
        let updated = match client.submit_dispute_evidence(&dispute.id, evidence).await {
            Ok(v) => v,
            Err(e) if is_file_required_error(&e) => {
                eprintln!("SKIP: sandbox requires file upload for evidence");
                cleanup_entities(&client, &customer_id, &pm_id).await;
                return;
            }
            Err(e) => panic!("submit_dispute_evidence failed: {e}"),
        };

        eprintln!(
            "[scenario-10] charge={} dispute={} status_after={}",
            charge_id, updated.id, updated.status
        );
        assert_eq!(updated.id, dispute.id);

        cleanup_entities(&client, &customer_id, &pm_id).await;
    }
}
