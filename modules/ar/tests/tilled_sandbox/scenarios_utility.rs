//! Sandbox scenarios: utility endpoints (health, product codes, platform fee refunds).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    #[tokio::test]
    async fn scenario_util1_health_check() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let health = retry
            .execute(|| client.get_health())
            .await
            .expect("get_health should succeed");

        eprintln!(
            "[scenario-util1] health: status={:?}, version={:?}, env={:?}",
            health.status, health.version, health.environment
        );
    }

    #[tokio::test]
    async fn scenario_util2_list_product_codes() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_product_codes(None))
            .await
            .expect("list_product_codes should succeed");

        eprintln!(
            "[scenario-util2] product codes: total={:?}, items={}",
            list.total,
            list.items.len()
        );

        for code in &list.items {
            assert!(!code.id.is_empty(), "product code ID must be non-empty");
            eprintln!(
                "[scenario-util2] code: id={}, name={:?}, type={:?}",
                code.id, code.name, code.payment_method_type
            );
        }
    }

    #[tokio::test]
    async fn scenario_util3_list_product_codes_partner() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let list = retry
            .execute(|| client.list_product_codes(None))
            .await
            .expect("list_product_codes (partner) should succeed");

        eprintln!(
            "[scenario-util3] product codes (partner): total={:?}, items={}",
            list.total,
            list.items.len()
        );
    }

    #[tokio::test]
    async fn scenario_util4_platform_fee_refund_probe() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // First find a platform fee to probe
        let fees = retry
            .execute(|| client.list_platform_fees(None))
            .await
            .expect("list_platform_fees should succeed");

        let fee = match fees.items.first() {
            Some(f) => f,
            None => {
                eprintln!("[scenario-util4] SKIP: no platform fees in sandbox");
                return;
            }
        };

        // Probe a nonexistent refund — expect 404
        let result = client
            .get_platform_fee_refund(&fee.id, "pfr_nonexistent_probe")
            .await;

        match result {
            Ok(refund) => {
                eprintln!(
                    "[scenario-util4] unexpectedly found refund: id={}, amount={:?}",
                    refund.id, refund.amount
                );
            }
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) => {
                eprintln!(
                    "[scenario-util4] fee refund probe: {} — {}",
                    status_code, message
                );
                assert!(
                    (400..500).contains(&status_code),
                    "expected 4xx for nonexistent refund, got {status_code}"
                );
            }
            Err(e) => panic!("get_platform_fee_refund failed unexpectedly: {e}"),
        }
    }
}
