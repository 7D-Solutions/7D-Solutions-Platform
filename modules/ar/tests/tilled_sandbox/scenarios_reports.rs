//! Sandbox scenarios: report runs (partner scope).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::try_partner_client;
    use ar_rs::tilled::report_runs::{CreateReportRunParams, ReportRunParameters};

    #[tokio::test]
    async fn scenario_rr1_create_and_list_report_runs() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Create a report run
        let params = CreateReportRunParams {
            report_type: "payments_summary_1".to_string(),
            parameters: ReportRunParameters {
                start_at: "2026-01-01T00:00:00.000Z".to_string(),
                end_at: "2026-02-01T00:00:00.000Z".to_string(),
            },
        };
        let created = retry
            .execute(|| client.create_report_run(&params))
            .await
            .expect("create_report_run should succeed");

        assert!(!created.id.is_empty(), "report run ID must be non-empty");
        assert_eq!(created.report_type, "payments_summary_1");
        assert!(
            created.status == "queued" || created.status == "finished",
            "status should be queued or finished, got: {}",
            created.status
        );
        eprintln!(
            "[scenario-rr1] created report run: id={}, status={}",
            created.id, created.status
        );

        // List report runs
        let list = retry
            .execute(|| client.list_report_runs(None))
            .await
            .expect("list_report_runs should succeed");

        assert!(
            !list.items.is_empty(),
            "should have at least one report run"
        );
        eprintln!(
            "[scenario-rr1] report runs: total={:?}, items={}",
            list.total,
            list.items.len()
        );
    }

    #[tokio::test]
    async fn scenario_rr2_get_report_run() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Create one first so we have a known ID
        let params = CreateReportRunParams {
            report_type: "fees_summary_1".to_string(),
            parameters: ReportRunParameters {
                start_at: "2026-01-01T00:00:00.000Z".to_string(),
                end_at: "2026-02-01T00:00:00.000Z".to_string(),
            },
        };
        let created = retry
            .execute(|| client.create_report_run(&params))
            .await
            .expect("create_report_run should succeed");

        // Get it by ID
        let fetched = retry
            .execute(|| client.get_report_run(&created.id))
            .await
            .expect("get_report_run should succeed");

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.report_type, "fees_summary_1");
        eprintln!(
            "[scenario-rr2] fetched report run: id={}, status={}",
            fetched.id, fetched.status
        );
    }
}
