/// Integration tests for TTP billing runs: service agreements, one-time charges,
/// and collect_parties_to_bill.
///
/// Requires DATABASE_URL pointing at a running TTP Postgres instance.
/// Run with: cargo test -p ttp-rs --test billing_run_integration -- --ignored
use sqlx::PgPool;
use uuid::Uuid;

use ttp_rs::domain::billing_repo::{collect_parties_to_bill, fetch_run_summary};

/// Connect to the TTP test database.
async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string());
    let pool = PgPool::connect(&url).await.expect("connect TTP test db");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    pool
}

/// Clean up all test data for a specific tenant.
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query(
        "DELETE FROM ttp_billing_run_items WHERE run_id IN \
         (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM ttp_billing_runs WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_one_time_charges WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_service_agreements WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_customers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_metering_events WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_metering_pricing WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Seed a customer + service agreement for a party.
async fn seed_agreement(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
    plan_code: &str,
    amount_minor: i64,
) {
    sqlx::query(
        "INSERT INTO ttp_customers (tenant_id, party_id, status) \
         VALUES ($1, $2, 'active') ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(pool)
    .await
    .expect("seed customer");

    sqlx::query(
        r#"INSERT INTO ttp_service_agreements
           (tenant_id, party_id, plan_code, amount_minor, currency, effective_from)
           VALUES ($1, $2, $3, $4, 'usd', '2026-01-01')"#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .bind(plan_code)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("seed agreement");
}

/// Seed a one-time charge for a party.
async fn seed_charge(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
    amount_minor: i64,
    description: &str,
) -> Uuid {
    let charge_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO ttp_one_time_charges
           (charge_id, tenant_id, party_id, description, amount_minor, currency, status)
           VALUES ($1, $2, $3, $4, $5, 'usd', 'pending')"#,
    )
    .bind(charge_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(description)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("seed charge");
    charge_id
}

/// Create a billing run record and return the run_id.
async fn create_run(pool: &PgPool, tenant_id: Uuid, period: &str) -> Uuid {
    let run_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO ttp_billing_runs (run_id, tenant_id, billing_period, status, idempotency_key)
           VALUES ($1, $2, $3, 'pending', $4)"#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(period)
    .bind(format!("test-key-{}", run_id))
    .execute(pool)
    .await
    .expect("create billing run");
    run_id
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn collect_parties_agreement_only() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_a, "starter", 9900).await;
    seed_agreement(&pool, tenant_id, party_b, "pro", 29900).await;

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert_eq!(parties.len(), 2, "two parties with agreements");

    let pa = parties
        .iter()
        .find(|p| p.party_id == party_a)
        .expect("party_a");
    assert_eq!(pa.total_amount_minor, 9900);
    assert_eq!(pa.currency, "usd");
    assert!(pa.charge_ids.is_empty());
    assert!(pa.trace_hash.is_none());

    let pb = parties
        .iter()
        .find(|p| p.party_id == party_b)
        .expect("party_b");
    assert_eq!(pb.total_amount_minor, 29900);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn collect_parties_with_one_time_charges() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // Agreement + two one-time charges for the same party
    seed_agreement(&pool, tenant_id, party_id, "starter", 9900).await;
    let c1 = seed_charge(&pool, tenant_id, party_id, 500, "Setup fee").await;
    let c2 = seed_charge(&pool, tenant_id, party_id, 1000, "Overage").await;

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert_eq!(parties.len(), 1, "one party");
    let p = &parties[0];
    // Agreement (9900) + charges (500 + 1000) = 11400
    assert_eq!(p.total_amount_minor, 11400);
    assert_eq!(p.charge_ids.len(), 2);
    assert!(p.charge_ids.contains(&c1));
    assert!(p.charge_ids.contains(&c2));

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn collect_parties_charges_only_no_agreement() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // Customer exists but no agreement — just a one-time charge
    sqlx::query(
        "INSERT INTO ttp_customers (tenant_id, party_id, status) \
         VALUES ($1, $2, 'active') ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(&pool)
    .await
    .expect("seed customer");

    let charge_id = seed_charge(&pool, tenant_id, party_id, 2500, "One-off service").await;

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].total_amount_minor, 2500);
    assert_eq!(parties[0].charge_ids, vec![charge_id]);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn collect_parties_skips_already_invoiced() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_a = Uuid::new_v4();
    let party_b = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_a, "starter", 9900).await;
    seed_agreement(&pool, tenant_id, party_b, "pro", 29900).await;

    let run_id = create_run(&pool, tenant_id, "2026-02").await;

    // Simulate party_a already invoiced
    sqlx::query(
        r#"INSERT INTO ttp_billing_run_items
           (run_id, party_id, ar_invoice_id, amount_minor, currency, status)
           VALUES ($1, $2, $3, 9900, 'usd', 'invoiced')"#,
    )
    .bind(run_id)
    .bind(party_a)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("seed invoiced item");

    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    // Only party_b should appear
    assert_eq!(parties.len(), 1, "only non-invoiced party returned");
    assert_eq!(parties[0].party_id, party_b);

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn collect_parties_empty_tenant() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert!(parties.is_empty(), "no parties for empty tenant");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn collect_parties_suspended_agreement_excluded() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    seed_agreement(&pool, tenant_id, party_id, "starter", 9900).await;

    // Suspend the agreement
    sqlx::query("UPDATE ttp_service_agreements SET status = 'suspended' WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("suspend agreement");

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert!(parties.is_empty(), "suspended agreements excluded");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn fetch_run_summary_returns_correct_totals() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let run_id = create_run(&pool, tenant_id, "2026-03").await;

    // Insert two invoiced items
    for (party_id, amount) in [(Uuid::new_v4(), 10000i64), (Uuid::new_v4(), 5000i64)] {
        sqlx::query(
            r#"INSERT INTO ttp_billing_run_items
               (run_id, party_id, ar_invoice_id, amount_minor, currency, status)
               VALUES ($1, $2, $3, $4, 'usd', 'invoiced')"#,
        )
        .bind(run_id)
        .bind(party_id)
        .bind(Uuid::new_v4())
        .bind(amount)
        .execute(&pool)
        .await
        .expect("insert item");
    }

    let (parties_billed, total, currency) = fetch_run_summary(&pool, run_id)
        .await
        .expect("fetch summary");

    assert_eq!(parties_billed, 2);
    assert_eq!(total, 15000);
    assert_eq!(currency, "usd");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn fetch_run_summary_empty_run() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let run_id = create_run(&pool, tenant_id, "2026-04").await;

    let (parties_billed, total, currency) = fetch_run_summary(&pool, run_id)
        .await
        .expect("fetch summary");

    assert_eq!(parties_billed, 0);
    assert_eq!(total, 0);
    assert_eq!(currency, "usd");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn multiple_agreements_same_party_summed() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // Same party, two active agreements
    seed_agreement(&pool, tenant_id, party_id, "base", 5000).await;
    sqlx::query(
        r#"INSERT INTO ttp_service_agreements
           (tenant_id, party_id, plan_code, amount_minor, currency, effective_from)
           VALUES ($1, $2, 'addon', 2000, 'usd', '2026-01-01')"#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(&pool)
    .await
    .expect("seed second agreement");

    let run_id = create_run(&pool, tenant_id, "2026-02").await;
    let parties = collect_parties_to_bill(&pool, tenant_id, run_id)
        .await
        .expect("collect parties");

    assert_eq!(parties.len(), 1);
    assert_eq!(parties[0].total_amount_minor, 7000, "5000 + 2000 = 7000");

    cleanup(&pool, tenant_id).await;
}
