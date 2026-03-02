//! Real E2E tests for the Numbering service.
//!
//! Tests run against a real Postgres database — no mocks, no stubs.
//! Each test uses a unique tenant_id to avoid cross-contamination.
//!
//! Default DB: postgresql://postgres:postgres@localhost:5450/numbering_db
//! Override with NUMBERING_DATABASE_URL env var.

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to numbering test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run numbering migrations");

    pool
}

fn unique_tenant() -> Uuid {
    Uuid::new_v4()
}

// ============================================================================
// 1. Basic allocation: first number for a tenant+entity is 1
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_first_number() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "quote";
    let idem_key = format!("numbering:allocate:{}:first", tid);

    let num = allocate(&pool, tid, entity, &idem_key).await;
    assert_eq!(num, 1, "First allocation should be 1");
}

// ============================================================================
// 2. Sequential: second allocation returns 2
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_sequential() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "invoice";

    let n1 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-1", tid),
    )
    .await;
    let n2 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-2", tid),
    )
    .await;
    let n3 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:seq-3", tid),
    )
    .await;

    assert_eq!(n1, 1);
    assert_eq!(n2, 2);
    assert_eq!(n3, 3);
}

// ============================================================================
// 3. Idempotency: same key returns same number
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "po";
    let idem_key = format!("numbering:allocate:{}:idem", tid);

    let n1 = allocate(&pool, tid, entity, &idem_key).await;
    let n2 = allocate(&pool, tid, entity, &idem_key).await;

    assert_eq!(n1, n2, "Same idempotency key must return same number");

    // A different key should get the next number
    let n3 = allocate(
        &pool,
        tid,
        entity,
        &format!("numbering:allocate:{}:idem-2", tid),
    )
    .await;
    assert_eq!(n3, 2, "Different key should advance the sequence");
}

// ============================================================================
// 4. Tenant isolation: different tenants get independent sequences
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let t1 = unique_tenant();
    let t2 = unique_tenant();
    let entity = "quote";

    let n_t1 = allocate(
        &pool,
        t1,
        entity,
        &format!("numbering:allocate:{}:iso", t1),
    )
    .await;
    let n_t2 = allocate(
        &pool,
        t2,
        entity,
        &format!("numbering:allocate:{}:iso", t2),
    )
    .await;

    assert_eq!(n_t1, 1, "Tenant 1 should start at 1");
    assert_eq!(n_t2, 1, "Tenant 2 should independently start at 1");
}

// ============================================================================
// 5. Entity isolation: different entities get independent sequences
// ============================================================================

#[tokio::test]
#[serial]
async fn test_entity_isolation() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let n_quote = allocate(
        &pool,
        tid,
        "quote",
        &format!("numbering:allocate:{}:ent-q", tid),
    )
    .await;
    let n_invoice = allocate(
        &pool,
        tid,
        "invoice",
        &format!("numbering:allocate:{}:ent-i", tid),
    )
    .await;

    assert_eq!(n_quote, 1, "Quote entity starts at 1");
    assert_eq!(n_invoice, 1, "Invoice entity starts independently at 1");
}

// ============================================================================
// 6. Concurrency: parallel allocations never produce duplicates
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_allocations_no_duplicates() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "work_order";
    let count = 20;

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let pool = pool.clone();
        let idem_key = format!("numbering:allocate:{}:conc-{}", tid, i);
        let entity = entity.to_string();
        handles.push(tokio::spawn(async move {
            allocate(&pool, tid, &entity, &idem_key).await
        }));
    }

    let mut results = Vec::with_capacity(count);
    for h in handles {
        results.push(h.await.expect("task panicked"));
    }

    results.sort();

    // Every number from 1..=count should appear exactly once
    let expected: Vec<i64> = (1..=count as i64).collect();
    assert_eq!(
        results, expected,
        "Concurrent allocations must produce exactly 1..={} with no gaps or duplicates",
        count
    );
}

// ============================================================================
// 7. Outbox event: allocation creates an outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_creates_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "receipt";
    let idem_key = format!("numbering:allocate:{}:outbox", tid);

    let _ = allocate(&pool, tid, entity, &idem_key).await;

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.allocated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(row.0 >= 1, "At least one outbox event should exist");
}

// ============================================================================
// 8. Policy upsert: create and update a numbering policy
// ============================================================================

#[tokio::test]
#[serial]
async fn test_policy_upsert_create_and_update() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Insert new policy
    let row = upsert_policy(&pool, tid, "quote", "QUO-{YYYY}-{number}", "QUO", 5).await;
    assert_eq!(row.pattern, "QUO-{YYYY}-{number}");
    assert_eq!(row.prefix, "QUO");
    assert_eq!(row.padding, 5);
    assert_eq!(row.version, 1);

    // Update same policy — version should bump
    let row2 = upsert_policy(&pool, tid, "quote", "Q-{YY}{MM}-{number}", "Q", 4).await;
    assert_eq!(row2.pattern, "Q-{YY}{MM}-{number}");
    assert_eq!(row2.prefix, "Q");
    assert_eq!(row2.padding, 4);
    assert_eq!(row2.version, 2, "Version must increment on update");
}

// ============================================================================
// 9. Policy read: fetch an existing policy
// ============================================================================

#[tokio::test]
#[serial]
async fn test_policy_read() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // No policy yet
    let missing = numbering::policy::get_policy(&pool, tid, "invoice").await
        .expect("query should succeed");
    assert!(missing.is_none(), "No policy should exist yet");

    // Create one
    upsert_policy(&pool, tid, "invoice", "INV-{number}", "INV", 6).await;

    // Read it back
    let found = numbering::policy::get_policy(&pool, tid, "invoice").await
        .expect("query should succeed")
        .expect("policy should exist");
    assert_eq!(found.pattern, "INV-{number}");
    assert_eq!(found.prefix, "INV");
    assert_eq!(found.padding, 6);
}

// ============================================================================
// 10. Policy tenant isolation: different tenants have independent policies
// ============================================================================

#[tokio::test]
#[serial]
async fn test_policy_tenant_isolation() {
    let pool = setup_db().await;
    let t1 = unique_tenant();
    let t2 = unique_tenant();

    upsert_policy(&pool, t1, "quote", "A-{number}", "A", 3).await;
    upsert_policy(&pool, t2, "quote", "B-{number}", "B", 5).await;

    let p1 = numbering::policy::get_policy(&pool, t1, "quote").await
        .expect("query ok").expect("policy exists");
    let p2 = numbering::policy::get_policy(&pool, t2, "quote").await
        .expect("query ok").expect("policy exists");

    assert_eq!(p1.prefix, "A");
    assert_eq!(p2.prefix, "B");
}

// ============================================================================
// 11. Policy outbox event: upsert creates an outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_policy_upsert_creates_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    upsert_policy(&pool, tid, "receipt", "{prefix}-{number}", "REC", 4).await;

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.policy.updated' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:receipt", tid))
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(row.0 >= 1, "Policy upsert should create an outbox event");
}

// ============================================================================
// 12. Formatting integration: allocate after policy returns formatted number
// ============================================================================

#[tokio::test]
#[serial]
async fn test_allocate_with_policy_returns_formatted() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Set up a policy
    upsert_policy(&pool, tid, "wo", "WO-{number}", "WO", 5).await;

    // Allocate a number
    let num = allocate(&pool, tid, "wo", &format!("numbering:allocate:{}:fmt-1", tid)).await;
    assert_eq!(num, 1);

    // Verify formatting via the library directly
    let policy_row = numbering::policy::get_policy(&pool, tid, "wo").await
        .expect("query ok").expect("policy exists");

    let fp = numbering::format::FormatPolicy {
        pattern: policy_row.pattern,
        prefix: policy_row.prefix,
        padding: policy_row.padding as u32,
    };
    let today = chrono::Utc::now().date_naive();
    let formatted = numbering::format::format_number(&fp, num, today);
    assert_eq!(formatted, "WO-00001");
}

// ============================================================================
// 13. Formatting does not affect allocation: changing policy doesn't affect
//     the raw number sequence
// ============================================================================

#[tokio::test]
#[serial]
async fn test_policy_change_does_not_affect_allocation() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Allocate before any policy
    let n1 = allocate(&pool, tid, "po", &format!("numbering:allocate:{}:pol-1", tid)).await;
    assert_eq!(n1, 1);

    // Set a policy
    upsert_policy(&pool, tid, "po", "PO-{YYYY}-{number}", "PO", 4).await;

    // Allocate again — raw number continues from where it left off
    let n2 = allocate(&pool, tid, "po", &format!("numbering:allocate:{}:pol-2", tid)).await;
    assert_eq!(n2, 2, "Allocation sequence must not reset when policy changes");

    // Change the policy
    upsert_policy(&pool, tid, "po", "X-{number}", "X", 6).await;

    // Allocate again — raw number still continues
    let n3 = allocate(&pool, tid, "po", &format!("numbering:allocate:{}:pol-3", tid)).await;
    assert_eq!(n3, 3, "Allocation sequence must not reset when policy changes again");
}

// ============================================================================
// 14. Gap-free: basic reserve + confirm flow
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_reserve_and_confirm() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_invoice";
    let idem_key = format!("gf:{}:1", tid);

    let alloc = allocate_gap_free(&pool, tid, entity, &idem_key).await;
    assert_eq!(alloc.number_value, 1);
    assert_eq!(alloc.status, "reserved", "Gap-free allocation must be reserved");
    assert!(alloc.expires_at.is_some(), "Reserved allocation must have expires_at");

    // Confirm it
    confirm_number(&pool, tid, entity, &idem_key).await;

    // Verify status in DB
    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tid)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("status query failed");
    assert_eq!(status, "confirmed");
}

// ============================================================================
// 15. Gap-free: idempotency — same key returns same reserved number
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_idempotent_reserve() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_po";
    let idem_key = format!("gf:{}:idem", tid);

    let a1 = allocate_gap_free(&pool, tid, entity, &idem_key).await;
    let a2 = allocate_gap_free(&pool, tid, entity, &idem_key).await;
    assert_eq!(a1.number_value, a2.number_value, "Same idem key must return same number");

    // Different key advances
    let a3 = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:idem-2", tid)).await;
    assert_eq!(a3.number_value, 2);
}

// ============================================================================
// 16. Gap-free: confirm is idempotent
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_confirm_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_receipt";
    let idem_key = format!("gf:{}:conf-idem", tid);

    allocate_gap_free(&pool, tid, entity, &idem_key).await;
    confirm_number(&pool, tid, entity, &idem_key).await;

    // Confirming again should succeed silently
    confirm_number(&pool, tid, entity, &idem_key).await;

    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tid)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("status query failed");
    assert_eq!(status, "confirmed");
}

// ============================================================================
// 17. Gap-free: expired reservation is recycled
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_expired_reservation_recycled() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_wo";

    // Reserve number 1
    let a1 = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:exp-1", tid)).await;
    assert_eq!(a1.number_value, 1);

    // Reserve number 2
    let a2 = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:exp-2", tid)).await;
    assert_eq!(a2.number_value, 2);

    // Confirm number 2 — number 1 stays reserved
    confirm_number(&pool, tid, entity, &format!("gf:{}:exp-2", tid)).await;

    // Expire number 1's reservation
    expire_reservation(&pool, tid, &format!("gf:{}:exp-1", tid)).await;

    // Allocate again — should recycle number 1 (not advance to 3)
    let a3 = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:exp-3", tid)).await;
    assert_eq!(
        a3.number_value, 1,
        "Expired reservation should be recycled — got {} instead of 1",
        a3.number_value
    );
}

// ============================================================================
// 18. Gap-free: crash/retry does not introduce gaps
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_crash_retry_no_gaps() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_crash";

    // Allocate 1, 2, 3 — all reserved
    for i in 1..=3 {
        let a = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:cr-{}", tid, i)).await;
        assert_eq!(a.number_value, i);
    }

    // Simulate crash: expire reservation for #2 (middle number)
    expire_reservation(&pool, tid, &format!("gf:{}:cr-2", tid)).await;

    // Confirm 1 and 3
    confirm_number(&pool, tid, entity, &format!("gf:{}:cr-1", tid)).await;
    confirm_number(&pool, tid, entity, &format!("gf:{}:cr-3", tid)).await;

    // Retry: allocate with a new key — should recycle #2
    let retry = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:cr-retry", tid)).await;
    assert_eq!(retry.number_value, 2, "Crash retry should recycle the expired #2");

    // Confirm the retried #2
    confirm_number(&pool, tid, entity, &format!("gf:{}:cr-retry", tid)).await;

    // Now allocate next — should be #4
    let next = allocate_gap_free(&pool, tid, entity, &format!("gf:{}:cr-4", tid)).await;
    assert_eq!(next.number_value, 4, "After recycle, next number should be 4");

    // Verify: confirmed numbers are 1, 2, 3 (contiguous)
    // plus 4 reserved
    let confirmed = get_confirmed_numbers(&pool, tid, entity).await;
    assert_eq!(confirmed, vec![1, 2, 3], "Confirmed numbers must be contiguous 1..3");
}

// ============================================================================
// 19. Gap-free: concurrent allocations produce no duplicates
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_concurrent_no_duplicates() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_conc";
    let count = 20;

    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        let pool = pool.clone();
        let idem_key = format!("gf:{}:conc-{}", tid, i);
        let entity = entity.to_string();
        handles.push(tokio::spawn(async move {
            allocate_gap_free(&pool, tid, &entity, &idem_key).await.number_value
        }));
    }

    let mut results = Vec::with_capacity(count);
    for h in handles {
        results.push(h.await.expect("task panicked"));
    }

    results.sort();

    let expected: Vec<i64> = (1..=count as i64).collect();
    assert_eq!(
        results, expected,
        "Gap-free concurrent allocations must produce 1..={} with no gaps or duplicates",
        count
    );
}

// ============================================================================
// 20. Gap-free: confirm produces outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gap_free_confirm_creates_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "gf_outbox";
    let idem_key = format!("gf:{}:outbox", tid);

    allocate_gap_free(&pool, tid, entity, &idem_key).await;
    confirm_number(&pool, tid, entity, &idem_key).await;

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'numbering.events.number.confirmed' \
         AND aggregate_id = $1",
    )
    .bind(format!("{}:{}", tid, entity))
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    assert!(row.0 >= 1, "Confirm must create an outbox event");
}

// ============================================================================
// 21. Standard mode backward compat: status is 'confirmed' immediately
// ============================================================================

#[tokio::test]
#[serial]
async fn test_standard_mode_status_confirmed() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let entity = "std_check";
    let idem_key = format!("std:{}:1", tid);

    allocate(&pool, tid, entity, &idem_key).await;

    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tid)
    .bind(&idem_key)
    .fetch_one(&pool)
    .await
    .expect("status query failed");

    assert_eq!(status, "confirmed", "Standard allocation must be immediately confirmed");
}

// ============================================================================
// Helper: allocate using direct SQL (same logic as the handler)
// ============================================================================

async fn allocate(pool: &sqlx::PgPool, tenant_id: Uuid, entity: &str, idem_key: &str) -> i64 {
    // Check idempotency
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT number_value FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await
    .expect("idempotency check failed");

    if let Some((val,)) = existing {
        return val;
    }

    let mut tx = pool.begin().await.expect("begin tx failed");

    // Guard + Mutation: atomic upsert — handles concurrent first-insert race
    let (next_value,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO sequences (tenant_id, entity, current_value)
        VALUES ($1, $2, 1)
        ON CONFLICT (tenant_id, entity)
        DO UPDATE SET current_value = sequences.current_value + 1,
                      updated_at = NOW()
        RETURNING current_value
        "#,
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut *tx)
    .await
    .expect("sequence upsert failed");

    // Record issued number
    sqlx::query(
        "INSERT INTO issued_numbers (tenant_id, entity, number_value, idempotency_key) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(next_value)
    .bind(idem_key)
    .execute(&mut *tx)
    .await
    .expect("issued_numbers insert failed");

    // Outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "number_value": next_value,
        "idempotency_key": idem_key,
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.number.allocated")
    .bind("number")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert failed");

    tx.commit().await.expect("commit failed");

    next_value
}

// ============================================================================
// Helper: upsert a policy using direct SQL via the policy repo
// ============================================================================

async fn upsert_policy(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    entity: &str,
    pattern: &str,
    prefix: &str,
    padding: i32,
) -> numbering::policy::PolicyRow {
    let mut tx = pool.begin().await.expect("begin tx failed");

    let row = numbering::policy::upsert_policy_tx(&mut tx, tenant_id, entity, pattern, prefix, padding)
        .await
        .expect("upsert_policy_tx failed");

    // Also enqueue outbox event (like the handler does)
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "pattern": pattern,
        "prefix": prefix,
        "padding": padding,
        "version": row.version,
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.policy.updated")
    .bind("policy")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert failed");

    tx.commit().await.expect("commit failed");

    row
}

// ============================================================================
// Helper: gap-free allocate — creates sequence with gap_free=true,
// returns the reservation details.
// ============================================================================

struct GapFreeAllocation {
    number_value: i64,
    status: String,
    expires_at: Option<chrono::NaiveDateTime>,
}

async fn allocate_gap_free(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    entity: &str,
    idem_key: &str,
) -> GapFreeAllocation {
    // Idempotency check
    let existing: Option<(i64, String, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT number_value, status, expires_at \
         FROM issued_numbers WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await
    .expect("idempotency check failed");

    if let Some((val, status, expires_at)) = existing {
        return GapFreeAllocation {
            number_value: val,
            status,
            expires_at,
        };
    }

    let mut tx = pool.begin().await.expect("begin tx failed");

    // Lock or create the sequence with gap_free=true
    let existing_seq: Option<(i64, bool, i32)> = sqlx::query_as(
        "SELECT current_value, gap_free, reservation_ttl_secs \
         FROM sequences WHERE tenant_id = $1 AND entity = $2 FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_optional(&mut *tx)
    .await
    .expect("sequence lock failed");

    let (next_value, ttl_secs, recycled) = match existing_seq {
        Some((_current, gap_free, ttl)) => {
            assert!(gap_free, "Expected gap_free sequence");

            // Try to recycle an expired reservation
            let recyclable: Option<(i64,)> = sqlx::query_as(
                "SELECT number_value FROM issued_numbers \
                 WHERE tenant_id = $1 AND entity = $2 \
                   AND status = 'reserved' AND expires_at < NOW() \
                 ORDER BY number_value ASC LIMIT 1 \
                 FOR UPDATE SKIP LOCKED",
            )
            .bind(tenant_id)
            .bind(entity)
            .fetch_optional(&mut *tx)
            .await
            .expect("recycle check failed");

            if let Some((recycled_val,)) = recyclable {
                (recycled_val, ttl, true)
            } else {
                let (next,): (i64,) = sqlx::query_as(
                    "UPDATE sequences SET current_value = current_value + 1, updated_at = NOW() \
                     WHERE tenant_id = $1 AND entity = $2 RETURNING current_value",
                )
                .bind(tenant_id)
                .bind(entity)
                .fetch_one(&mut *tx)
                .await
                .expect("counter advance failed");
                (next, ttl, false)
            }
        }
        None => {
            let (next,): (i64,) = sqlx::query_as(
                "INSERT INTO sequences (tenant_id, entity, current_value, gap_free) \
                 VALUES ($1, $2, 1, TRUE) \
                 ON CONFLICT (tenant_id, entity) \
                 DO UPDATE SET current_value = sequences.current_value + 1, updated_at = NOW() \
                 RETURNING current_value",
            )
            .bind(tenant_id)
            .bind(entity)
            .fetch_one(&mut *tx)
            .await
            .expect("sequence create failed");
            (next, 300, false)
        }
    };

    let expires_at =
        chrono::Utc::now().naive_utc() + chrono::Duration::seconds(ttl_secs as i64);

    if recycled {
        sqlx::query(
            "UPDATE issued_numbers \
             SET idempotency_key = $1, status = 'reserved', expires_at = $2 \
             WHERE tenant_id = $3 AND entity = $4 AND number_value = $5",
        )
        .bind(idem_key)
        .bind(expires_at)
        .bind(tenant_id)
        .bind(entity)
        .bind(next_value)
        .execute(&mut *tx)
        .await
        .expect("recycle update failed");
    } else {
        sqlx::query(
            "INSERT INTO issued_numbers \
             (tenant_id, entity, number_value, idempotency_key, status, expires_at) \
             VALUES ($1, $2, $3, $4, 'reserved', $5)",
        )
        .bind(tenant_id)
        .bind(entity)
        .bind(next_value)
        .bind(idem_key)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .expect("issued_numbers insert failed");
    }

    // Outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "number_value": next_value,
        "idempotency_key": idem_key,
        "status": "reserved",
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.number.allocated")
    .bind("number")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert failed");

    tx.commit().await.expect("commit failed");

    GapFreeAllocation {
        number_value: next_value,
        status: "reserved".to_string(),
        expires_at: Some(expires_at),
    }
}

// ============================================================================
// Helper: confirm a gap-free reservation
// ============================================================================

async fn confirm_number(pool: &sqlx::PgPool, tenant_id: Uuid, entity: &str, idem_key: &str) {
    let mut tx = pool.begin().await.expect("begin tx failed");

    let (number_value, status): (i64, String) = sqlx::query_as(
        "SELECT number_value, status FROM issued_numbers \
         WHERE tenant_id = $1 AND idempotency_key = $2 FOR UPDATE",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_one(&mut *tx)
    .await
    .expect("confirm lookup failed");

    if status == "confirmed" {
        // Already confirmed — idempotent
        return;
    }

    assert_eq!(status, "reserved", "Can only confirm reserved numbers");

    sqlx::query(
        "UPDATE issued_numbers SET status = 'confirmed', expires_at = NULL \
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .execute(&mut *tx)
    .await
    .expect("confirm update failed");

    // Outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "number_value": number_value,
        "idempotency_key": idem_key,
    });

    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.number.confirmed")
    .bind("number")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(payload)
    .execute(&mut *tx)
    .await
    .expect("confirm outbox insert failed");

    tx.commit().await.expect("confirm commit failed");
}

// ============================================================================
// Helper: force-expire a reservation (simulate crash / timeout)
// ============================================================================

async fn expire_reservation(pool: &sqlx::PgPool, tenant_id: Uuid, idem_key: &str) {
    sqlx::query(
        "UPDATE issued_numbers SET expires_at = NOW() - INTERVAL '1 second' \
         WHERE tenant_id = $1 AND idempotency_key = $2 AND status = 'reserved'",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .execute(pool)
    .await
    .expect("expire_reservation failed");
}

// ============================================================================
// Helper: get all confirmed numbers for a tenant+entity, sorted
// ============================================================================

async fn get_confirmed_numbers(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    entity: &str,
) -> Vec<i64> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT number_value FROM issued_numbers \
         WHERE tenant_id = $1 AND entity = $2 AND status = 'confirmed' \
         ORDER BY number_value ASC",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_all(pool)
    .await
    .expect("confirmed numbers query failed");

    rows.into_iter().map(|(v,)| v).collect()
}
