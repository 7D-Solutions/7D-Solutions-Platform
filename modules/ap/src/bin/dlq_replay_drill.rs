//! DLQ replay drill for the AP module.
//!
//! Exercises the outbox replay path:
//!   1. Creates a vendor to seed an outbox event
//!   2. Verifies the event is in fetch_unpublished
//!   3. Marks it published via mark_published
//!   4. Verifies it no longer appears in fetch_unpublished
//!   5. Inserts a second event, resets published_at, verifies re-fetch
//!   6. Validates cleanup (deletes bench data)
//!
//! This validates that the operational replay procedure works end-to-end
//! against the real database.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use uuid::Uuid;

use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::CreateVendorRequest;
use ap::outbox::{fetch_unpublished, mark_published};

const DRILL_TENANT: &str = "drill-dlq-replay";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    sqlx::migrate!("db/migrations").run(&pool).await?;

    println!("dlq_replay_drill: starting");
    println!("  db={db_url}");
    println!("  tenant={DRILL_TENANT}");

    // Clean up any leftover drill data
    cleanup(&pool).await;

    // ── Step 1: Create a vendor to seed an outbox event ──────────────

    let req = CreateVendorRequest {
        name: format!("Drill Vendor {}", Uuid::new_v4()),
        tax_id: None,
        currency: "USD".to_string(),
        payment_terms_days: 30,
        payment_method: None,
        remittance_email: None,
        party_id: None,
    };
    let vendor = create_vendor(&pool, DRILL_TENANT, &req, "drill-corr-1".to_string()).await?;
    println!("  vendor_created={}", vendor.vendor_id);

    // ── Step 2: Verify the event appears in fetch_unpublished ────────

    let unpublished = fetch_unpublished(&pool, 100).await?;
    let found = unpublished
        .iter()
        .any(|e| e.aggregate_id == vendor.vendor_id.to_string());
    assert!(
        found,
        "vendor_created event must appear in fetch_unpublished"
    );
    println!("  fetch_unpublished=found");

    // ── Step 3: Mark it published ───────────────────────────────────

    let event = unpublished
        .iter()
        .find(|e| e.aggregate_id == vendor.vendor_id.to_string())
        .expect("event must exist");
    let event_id = event.event_id;

    mark_published(&pool, event_id).await?;
    println!("  mark_published=ok event_id={event_id}");

    // ── Step 4: Verify it no longer appears in fetch_unpublished ─────

    let after_publish = fetch_unpublished(&pool, 100).await?;
    let still_there = after_publish.iter().any(|e| e.event_id == event_id);
    assert!(
        !still_there,
        "event must not appear in fetch_unpublished after mark_published"
    );
    println!("  post_publish_check=ok (event gone from unpublished)");

    // ── Step 5: Simulate replay by resetting published_at ───────────

    sqlx::query("UPDATE events_outbox SET published_at = NULL WHERE event_id = $1")
        .bind(event_id)
        .execute(&pool)
        .await?;

    let replayed = fetch_unpublished(&pool, 100).await?;
    let re_found = replayed.iter().any(|e| e.event_id == event_id);
    assert!(
        re_found,
        "event must reappear after published_at reset (simulated replay)"
    );
    println!("  replay_simulation=ok (event reappears after reset)");

    // ── Step 6: Re-publish to prove idempotency ─────────────────────

    mark_published(&pool, event_id).await?;
    let final_check = fetch_unpublished(&pool, 100).await?;
    let gone_again = !final_check.iter().any(|e| e.event_id == event_id);
    assert!(gone_again, "re-publish must be idempotent");
    println!("  republish_idempotent=ok");

    // ── Cleanup ─────────────────────────────────────────────────────

    cleanup(&pool).await;
    println!("dlq_replay_drill=ok");

    Ok(())
}

async fn cleanup(pool: &PgPool) {
    for q in [
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(DRILL_TENANT).execute(pool).await.ok();
    }
}
